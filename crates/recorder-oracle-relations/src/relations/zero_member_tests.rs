use super::*;
use replay_data::{
    rows::FactionMemberRow, rows::Recorded, rows::ReservedCountryNamingRow, rows::TableRows,
};
use replay_frame::frame::TableView;

const FACTION: u64 = (7 << 32) | 0x128;

fn fixture() -> (DecodedFrame, Save) {
    let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
    let row = FactionRow {
        id: FACTION,
        color_alpha: 1.0_f32.to_bits(),
        ..Default::default()
    };
    save.factions.insert(
        FACTION,
        content_reader_save::Faction {
            goals: Default::default(),
            companions: Default::default(),
            id: FACTION,
            name: None,
            members: vec![b"GER".to_vec(), vec![b'R', 0, 0xff]],
            icon: None,
            template: None,
            color: Some(saved_colour([
                row.color_red,
                row.color_green,
                row.color_blue,
                row.color_alpha,
            ])),
            color_overridden: Some(false),
            theatres: Vec::new(),
            advisor_slots: Vec::new(),
        },
    );
    *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(vec![row]));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 102,
                faction: FACTION,
                ordinal: 0,
            },
            FactionMemberRow {
                country: 0,
                faction: FACTION,
                ordinal: 1,
            },
        ]));
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: vec![b'R', 0, 0xff],
                faction: Recorded::Value(None),
                ..Default::default()
            },
        ]));
    (frame, save)
}

fn run(frame: &DecodedFrame, save: &Save) -> io::Result<Report> {
    let context = super::super::country_context(frame)?;
    let mut report = Report::default();
    compare_factions(frame, save, &context, &mut report)?;
    Ok(report)
}

fn members(frame: &mut DecodedFrame) -> &mut Vec<FactionMemberRow> {
    let TableView::Complete(TableRows::FactionMembers(rows)) =
        &mut *frame.tables[TableId::FactionMembers.index()]
    else {
        unreachable!()
    };
    rows
}

#[test]
fn serialized_zero_tag_and_positive_alias_compare_exactly() {
    let (frame, save) = fixture();
    let report = run(&frame, &save).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.factions, 1);
    assert_eq!(report.faction_members, 2);
    assert_eq!(report.scalar_checks, 0);
}

#[test]
fn positive_only_members_do_not_demand_reserved_naming() {
    let (mut frame, mut save) = fixture();
    members(&mut frame).pop();
    save.factions.get_mut(&FACTION).unwrap().members.pop();
    *frame.tables[TableId::ReservedCountryNaming.index()] = TableView::Unavailable {
        reason: "reserved root unreadable".into(),
    };
    assert!(run(&frame, &save).unwrap().differences.is_empty());
    members(&mut frame).push(FactionMemberRow {
        country: 0,
        faction: FACTION,
        ordinal: 1,
    });
    assert!(run(&frame, &save).is_err());
}

#[test]
fn missing_extra_reordered_and_changed_zero_members_remain_differences() {
    let (frame, save) = fixture();
    let mut missing = frame.clone();
    members(&mut missing).pop();
    let report = run(&missing, &save).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("members[1]"));

    let mut shorter_save = save.clone();
    shorter_save
        .factions
        .get_mut(&FACTION)
        .unwrap()
        .members
        .pop();
    assert_eq!(run(&frame, &shorter_save).unwrap().differences.len(), 1);

    let mut reordered_save = save.clone();
    reordered_save
        .factions
        .get_mut(&FACTION)
        .unwrap()
        .members
        .swap(0, 1);
    assert_eq!(run(&frame, &reordered_save).unwrap().differences.len(), 2);

    let mut changed_frame = frame;
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *changed_frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        unreachable!()
    };
    rows[0].tag[2] = 0xfe;
    assert!(run(&changed_frame, &save).is_err());
}

#[test]
fn duplicate_zero_members_and_noncontiguous_ordinals_are_rejected() {
    let (frame, save) = fixture();
    let mut duplicate = frame.clone();
    members(&mut duplicate).push(FactionMemberRow {
        country: 0,
        faction: FACTION,
        ordinal: 2,
    });
    assert!(run(&duplicate, &save).is_err());

    let mut duplicate_save = save.clone();
    let faction = duplicate_save.factions.get_mut(&FACTION).unwrap();
    faction.members.push(faction.members[1].clone());
    assert!(run(&frame, &duplicate_save).is_err());

    let mut skipped = frame.clone();
    members(&mut skipped)[1].ordinal = 2;
    assert!(run(&skipped, &save).is_err());

    let mut duplicate_ordinal = frame;
    members(&mut duplicate_ordinal)[1].ordinal = 0;
    assert!(run(&duplicate_ordinal, &save).is_err());
}

#[test]
fn reserved_tag_ambiguity_and_unrelated_missing_positive_country_are_rejected() {
    let (mut frame, save) = fixture();
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        unreachable!()
    };
    rows[0].tag = b"GER".to_vec();
    assert!(run(&frame, &save).is_err());

    let (frame, mut missing_positive) = fixture();
    missing_positive.countries.remove(b"GER".as_slice());
    assert!(run(&frame, &missing_positive).is_err());

    let (frame, mut unknown) = fixture();
    unknown.factions.get_mut(&FACTION).unwrap().members[1] = b"unrecorded".to_vec();
    assert!(run(&frame, &unknown).is_err());
}

#[test]
fn present_reserved_country_requires_admitted_physical_alias_and_routes_to_zero() {
    let (frame, mut save) = fixture();
    let tag = vec![b'R', 0, 0xff];
    save.countries
        .insert(tag.clone(), content_reader_save::Country::default());
    assert!(run(&frame, &save).is_err());
    save.country_emissions = Some(vec![content_reader_save::SavedCountryEmission {
        identity: content_reader_save::CountryEmissionIdentity {
            physical_slot: 1,
            own_id: 0,
            actual: content_reader_save::ActualCountry::Reserved,
            tag,
        },
        intel_present: false,
    }]);
    let report = run(&frame, &save).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.faction_members, 2);
    for field in [0, 1, 2, 3] {
        let mut wrong = save.clone();
        let identity = &mut wrong.country_emissions.as_mut().unwrap()[0].identity;
        match field {
            0 => identity.own_id = 7,
            1 => identity.actual = content_reader_save::ActualCountry::Canonical(7),
            2 => identity.physical_slot = 0,
            _ => identity.tag = b"OTHER".to_vec(),
        }
        assert!(run(&frame, &wrong).is_err());
    }
    save.factions
        .get_mut(&FACTION)
        .unwrap()
        .members
        .push(vec![b'R', 0, 0xff]);
    assert!(run(&frame, &save).is_err());
}
