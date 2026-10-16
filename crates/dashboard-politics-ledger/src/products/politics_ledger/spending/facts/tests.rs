use super::*;
use replay::frame::{TABLE_COUNT, TableView};

fn frame(hour: i32) -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn::<_, TABLE_COUNT, _>(|_| {
            TableView::Unavailable {
                reason: "fixture".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    }
}
fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    frame.tables[index] = TableView::Complete(rows).into();
}
fn mutation(hour: i32) -> LedgerMutation {
    LedgerMutation {
        country: 73,
        instance: 1,
        sequence: hour.to_string(),
        epoch: "1".into(),
        hour,
        read_hour: hour,
        currency: "Command".into(),
        cause: "LeaderPromotion".into(),
        operation: "Subtract".into(),
        site: "SpendCommandPower".into(),
        amount_raw: Some("-100000".into()),
        realized_delta_raw: "-100000".into(),
        before_raw: "100000".into(),
        after_raw: "0".into(),
        covered: true,
        item: LedgerItem {
            action: "Pay".into(),
            token: None,
            key: vec![],
            slot_key: vec![],
            character: None,
            object_kind: "Leader".into(),
            object_reference: Some(((12u64 << 32) | 5).to_string()),
            target_country: None,
            target_state: None,
            target_region: None,
            origin_kind: "None".into(),
            origin_token: None,
            origin_key: vec![],
        },
    }
}
#[test]
fn hourly_changing_leaders_over_three_months_share_one_event_fact() {
    let mut shared = Shared::default();
    let mut facts = Vec::new();
    let mut current = frame(0);
    set(
        &mut current,
        TableRows::Characters(vec![CharacterRow {
            id: 93,
            token: b"Recorded name".to_vec(),
            ..Default::default()
        }]),
    );
    // Keep the referenced leader and unrelated changing rows.
    let mut leaders: Vec<_> = (0..13)
        .map(|id| UnitLeaderRow {
            id,
            reference_type: 5,
            character_id: if id == 12 { 93 } else { id + 100 },
            category: 0,
            ..Default::default()
        })
        .collect();
    for hour in 0..90 * 24 {
        current.hour = hour;
        for leader in &mut leaders {
            leader.skill_required_experience = hour;
        }
        set(&mut current, TableRows::UnitLeaders(leaders.clone()));
        if hour % 24 == 0 {
            facts.push(event_facts(
                &mut shared,
                &mutation(hour),
                Some(&current),
                PowerCause::LeaderPromotion,
                None,
            ));
        }
    }
    let first = facts[0].character.as_ref().unwrap();
    assert_eq!(first.id, 93);
    assert_eq!(first.saved_name, "Recorded name");
    assert_eq!(first.leader_category, Some(0));
    assert!(
        facts
            .iter()
            .all(|fact| Arc::ptr_eq(fact.character.as_ref().unwrap(), first))
    );
}
#[test]
fn late_events_keep_recorded_key_without_borrowing_character_or_lexer() {
    let mut current = frame(2);
    set(
        &mut current,
        TableRows::Characters(vec![CharacterRow {
            id: 93,
            token: b"Later rename".to_vec(),
            ..Default::default()
        }]),
    );
    let mut row = mutation(1);
    row.item.character = Some(93);
    row.item.key = b"recorded_key".to_vec();
    let facts = event_facts(
        &mut Shared::default(),
        &row,
        Some(&current),
        PowerCause::LeaderPromotion,
        None,
    );
    assert_eq!(facts.token.as_deref(), Some("recorded_key"));
    assert!(facts.character.is_none());
}
#[test]
fn component_snapshots_use_recorded_modifier_and_organisation_identity() {
    let mut current = frame(24);
    set(
        &mut current,
        TableRows::ModifierDefinitions(vec![ModifierDefinitionRow {
            index: 0x26,
            name_key: b"modded_daily_cost_title".to_vec(),
            argument: b"recorded argument".to_vec(),
            ..Default::default()
        }]),
    );
    set(
        &mut current,
        TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 94,
            literal: b"modded_organisation".to_vec(),
        }]),
    );
    set(
        &mut current,
        TableRows::IndustrialOrganisations(vec![IndustrialOrganisationRow {
            organisation: 123,
            owner: 73,
            token: 94,
            name: b"modded_organisation_name".to_vec(),
            icon: b"modded_organisation_sprite".to_vec(),
            ..Default::default()
        }]),
    );
    set(
        &mut current,
        TableRows::Countries(vec![CountryRow {
            id: 73,
            canonical_slot: 1,
            tag: b"AAA".to_vec(),
            ..Default::default()
        }]),
    );
    set(
        &mut current,
        TableRows::CountryLinks(vec![CountryLinkRow {
            id: 73,
            canonical_slot: 1,
        }]),
    );
    let mut shared = Shared::default();
    let mut row = mutation(24);
    let facts = event_facts(
        &mut shared,
        &row,
        Some(&current),
        PowerCause::DailyGain,
        Some(PowerDailyCostKind::PoliticalPowerCostModifier),
    );
    assert_eq!(
        facts.modifier.as_ref().unwrap().name_key,
        b"modded_daily_cost_title"
    );
    row.item.object_kind = "Organisation".into();
    row.item.object_reference = Some("123".into());
    let facts = event_facts(
        &mut shared,
        &row,
        Some(&current),
        PowerCause::DailyGain,
        Some(PowerDailyCostKind::MioResearch),
    );
    let organisation = facts.organisation.as_ref().unwrap();
    assert_eq!(organisation.token, "modded_organisation");
    assert_eq!(organisation.name_key, "modded_organisation_name");
    assert_eq!(organisation.owner_tag.as_deref(), Some("AAA"));
}

#[test]
fn recorded_trigger_tables_cover_world_and_scope_reader_dependencies() {
    let selected: BTreeSet<_> = crate::triggers::RECORDED_TABLES
        .iter()
        .map(|table| format!("{table:?}"))
        .collect();
    let known: BTreeSet<_> = TableId::ALL
        .iter()
        .map(|table| format!("{table:?}"))
        .collect();
    let sources = [
        (
            "World",
            include_str!("../../../../../../dashboard-text/src/triggers/world.rs"),
        ),
        (
            "scope",
            include_str!("../../../../../../dashboard-text/src/scope.rs"),
        ),
        (
            "original tags",
            include_str!("../../../../../../dashboard-text/src/scope/original_tags.rs"),
        ),
        (
            "character scope",
            include_str!("../../../../../../dashboard-text/src/character_scope.rs"),
        ),
        (
            "state resources",
            include_str!("../../../../../../dashboard-text/src/state_resources.rs"),
        ),
        (
            "scripted text",
            include_str!("../../../../../../dashboard-text/src/scripted_text.rs"),
        ),
        (
            "scripted text entries",
            include_str!("../../../../../../dashboard-text/src/scripted_text/entries.rs"),
        ),
        (
            "scripted text properties",
            include_str!("../../../../../../dashboard-text/src/scripted_text/properties.rs"),
        ),
        (
            "scripted text balances",
            include_str!("../../../../../../dashboard-text/src/scripted_text/balance.rs"),
        ),
        (
            "scripted localisation seeds",
            include_str!("../../../../../../dashboard-text/src/triggers/scripted_localisation.rs"),
        ),
    ];
    for (label, source) in sources {
        // Production code ends at the first test-only item, whether it is compiled
        // for this crate's tests or also for the tests of crates above it.
        let source = source
            .split("#[cfg(test)]")
            .next()
            .unwrap()
            .split("#[cfg(any(test, feature = \"test-support\"))]")
            .next()
            .unwrap();
        for part in source.split("tables::").skip(1) {
            let name: String = part
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect();
            if known.contains(&name) {
                assert!(
                    selected.contains(&name),
                    "{label} reads {name}, missing from RECORDED_TABLES"
                );
            }
        }
    }
}

#[test]
fn refused_item_reasons_do_not_acquire_daily_hour_identity() {
    assert_eq!(
        undated_reason("item unavailable at hour 24"),
        undated_reason("item unavailable at hour 48")
    );
    assert_eq!(
        undated_reason("event-hour frame 24 is unavailable"),
        "event-hour frame is unavailable"
    );
    assert_eq!(
        undated_reason("character 24 has no template"),
        "character 24 has no template"
    );
}
