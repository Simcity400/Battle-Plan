use super::*;
use replay_data::{
    rows::CountryCounterInputRow, rows::FactionMemberRow, rows::ReservedCountryIntelSample,
    rows::ReservedReverseVisibilityInputs, rows::ReverseZeroVisibilityInputs, rows::TableId,
    rows::TableRows,
};
use replay_frame::frame::TableView;

const CUTOFFS: &[i64] = &[25_000, -500_000, 75_000, -200_000, 100_001, 500_000];

fn set<T: replay_data::rows::TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

fn install(frame: &mut DecodedFrame, cells: Vec<Option<ReverseZeroVisibilityInputs>>) {
    let sample_hour = frame.hour;
    set::<tables::ReservedCountryNaming>(
        frame,
        vec![ReservedCountryNamingRow {
            faction: Recorded::Value(Some(0)),
            reverse_visibility: Recorded::Value(Some(ReservedReverseVisibilityInputs {
                sample_hour: replay_data::rows::PacketHour::Stamped(sample_hour),
                cells,
            })),
            ..Default::default()
        }],
    );
}

fn reserved_mut(frame: &mut DecodedFrame) -> &mut ReservedCountryNamingRow {
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        panic!("fixture reserved singleton missing")
    };
    &mut rows[0]
}

fn cell(target: u32) -> Option<ReverseZeroVisibilityInputs> {
    Some(ReverseZeroVisibilityInputs {
        target: Some(target),
        war: Some([0, 0]),
        subject: Some(ZeroSubjectInputs::default()),
        military_access: None,
    })
}

fn subject(first: Option<u32>, second: Option<u32>) -> ZeroSubjectInputs {
    ZeroSubjectInputs {
        relation_type: Some(engine_executable::native_values::subject_relation::TYPE_VALUE),
        parties: Some([first, second]),
    }
}

fn certificate(frame: &mut DecodedFrame, sample_hour: i32, vector_count: u32) {
    reserved_mut(frame).country_intel_sample = Recorded::Value(Some(ReservedCountryIntelSample {
        sample_hour,
        vector_count,
    }));
}

fn cutoff(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    source: u32,
    target: u32,
    cutoffs: &[i64],
) -> Result<i32, &'static str> {
    let topology = Topology::default();
    let relations = Relations::default();
    Rebuild {
        frame,
        links,
        topology: &topology,
        controllers: BTreeMap::new(),
        intel: Intel {
            rows: complete::<tables::CountryIntel>(frame, "CountryIntel"),
            raws: raw_references(links),
        },
        relations: &relations,
        maximum: 100_000,
        cutoffs,
        bytes: BTreeMap::new(),
    }
    .cutoff(source, target)
}

#[test]
fn reverse_zero_empty_effective_cutoffs_precede_all_native_query_operands() {
    let (mut frame, _, _) = tests::fixture();
    *frame.tables[TableId::CountryIntel.index()] = TableView::Unavailable {
        reason: "getter is not reached".into(),
    };
    let links = BTreeMap::new();
    for cutoffs in [&[][..], &[123][..]] {
        assert_eq!(cutoff(&frame, &links, 999, 998, cutoffs), Ok(0));
    }
    assert_eq!(
        cutoff(&frame, &links, 999, 998, CUTOFFS),
        Err("visibility country link")
    );
}

#[test]
fn reverse_zero_uses_actual_receiver_identity_and_keeps_positive_target_alias_zero() {
    let (mut frame, mut links, _) = tests::fixture();
    links.insert(777, 0);
    assert_eq!(cutoff(&frame, &links, 777, 0, CUTOFFS), Ok(5));
    assert_eq!(cutoff(&frame, &links, 0, 0, CUTOFFS), Ok(5));
    assert_eq!(
        cutoff(&frame, &links, 777, 777, CUTOFFS),
        Err("ReservedCountryNaming")
    );
    install(
        &mut frame,
        vec![Some(ReverseZeroVisibilityInputs {
            war: Some([1, 0]),
            ..Default::default()
        })],
    );
    certificate(&mut frame, 0, 1);
    set::<tables::CountryIntel>(
        &mut frame,
        vec![CountryIntelRow {
            owner_country: 0,
            target_index: 0,
            army: 40_000,
            ..Default::default()
        }],
    );
    assert_eq!(cutoff(&frame, &links, 777, 777, CUTOFFS), Ok(-2));
    assert_eq!(cutoff(&frame, &links, 0, 777, CUTOFFS), Ok(-2));
    assert_eq!(cutoff(&frame, &links, 777, 0, CUTOFFS), Ok(5));
    assert_eq!(
        cutoff(&frame, &links, 0, 999, CUTOFFS),
        Err("visibility country link")
    );
}

#[test]
fn reverse_zero_slot_zero_is_independent_of_the_positive_country_census() {
    let (mut frame, mut links, _) = tests::fixture();
    links.insert(777, 0);
    install(&mut frame, vec![cell(0)]);
    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "positive census fault".into(),
    };
    *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "not reached".into(),
    };
    assert_eq!(reverse_zero_shared(&frame, &links, 0), Ok(false));
    assert_eq!(
        reverse_zero_shared(&frame, &links, 1),
        Err("ReservedCountryNaming.reverse_visibility cell capture refused")
    );
}

#[test]
fn reverse_zero_war_veto_precedes_faction_target_and_subject() {
    let (mut frame, links, _) = tests::fixture();
    install(
        &mut frame,
        vec![
            None,
            Some(ReverseZeroVisibilityInputs {
                war: Some([1, 0]),
                ..Default::default()
            }),
        ],
    );
    reserved_mut(&mut frame).faction = Recorded::NotRecorded;
    *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "not reached".into(),
    };
    assert_eq!(reverse_zero_shared(&frame, &links, 1), Ok(false));
    let Recorded::Value(Some(packet)) = &mut reserved_mut(&mut frame).reverse_visibility else {
        panic!()
    };
    packet.cells[1].as_mut().unwrap().war = None;
    assert_eq!(
        reverse_zero_shared(&frame, &links, 1),
        Err("ReservedCountryNaming.reverse_visibility war capture refused")
    );
    let Recorded::Value(Some(packet)) = &mut reserved_mut(&mut frame).reverse_visibility else {
        panic!()
    };
    packet.cells[1].as_mut().unwrap().war = Some([1, 9]);
    assert_eq!(
        reverse_zero_shared(&frame, &links, 1),
        Err("ReservedCountryNaming.faction not recorded")
    );
    for war in [[2, 0], [0, 1]] {
        assert_eq!(
            active_war(war),
            Err("visibility war has invalid native presence")
        );
    }
}

#[test]
fn reverse_zero_faction_shortcut_is_lazy_and_does_not_bypass_unknown_war() {
    let (mut frame, links, _) = tests::fixture();
    install(
        &mut frame,
        vec![
            cell(0),
            Some(ReverseZeroVisibilityInputs {
                war: Some([0, 0]),
                ..Default::default()
            }),
        ],
    );
    reserved_mut(&mut frame).faction = Recorded::Value(Some(7));
    *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "membership unknown".into(),
    };
    assert_eq!(reverse_zero_shared(&frame, &links, 0), Ok(true));
    assert_eq!(
        reverse_zero_shared(&frame, &links, 1),
        Err("FactionMembers")
    );
    set::<tables::FactionMembers>(
        &mut frame,
        vec![FactionMemberRow {
            country: 101,
            faction: 7,
            ordinal: 0,
        }],
    );
    assert_eq!(reverse_zero_shared(&frame, &links, 1), Ok(true));
    let Recorded::Value(Some(packet)) = &mut reserved_mut(&mut frame).reverse_visibility else {
        panic!()
    };
    packet.cells[1].as_mut().unwrap().war = None;
    assert_eq!(
        reverse_zero_shared(&frame, &links, 1),
        Err("ReservedCountryNaming.reverse_visibility war capture refused")
    );
}

#[test]
fn reverse_zero_subject_matches_actual_cell_target_by_ordered_raw_then_canonical_or() {
    let (mut frame, mut links, _) = tests::fixture();
    links.extend([(701, 1), (777, 0), (778, 0)]);
    install(
        &mut frame,
        vec![
            cell(0),
            Some(ReverseZeroVisibilityInputs {
                target: Some(701),
                war: Some([1, 9]),
                subject: Some(subject(Some(101), None)),
                military_access: None,
            }),
        ],
    );
    assert_eq!(reverse_zero_shared(&frame, &links, 1), Ok(true));
    assert_eq!(
        map_intel_subject(&subject(Some(999), None), &links, Ok(999)),
        Ok(true)
    );
    assert_eq!(
        map_intel_subject(&subject(None, Some(101)), &links, Ok(701)),
        Ok(true)
    );
    assert_eq!(
        map_intel_subject(&subject(Some(999), Some(101)), &links, Ok(701)),
        Ok(true)
    );
    assert_eq!(
        map_intel_subject(&subject(Some(777), None), &links, Ok(0)),
        Err("visibility subject party capture refused")
    );
    assert_eq!(
        map_intel_subject(&subject(Some(0), None), &links, Ok(0)),
        Ok(true)
    );
    assert_eq!(
        map_intel_subject(&subject(Some(777), None), &links, Ok(777)),
        Ok(true)
    );
    assert_eq!(
        map_intel_subject(&subject(Some(777), None), &links, Ok(778)),
        Ok(true)
    );
    assert_eq!(
        map_intel_subject(&subject(Some(101), Some(102)), &links, Ok(103)),
        Ok(false)
    );
    assert_eq!(
        map_intel_subject(&subject(None, Some(102)), &links, Ok(103)),
        Err("visibility subject party capture refused")
    );
}

#[test]
fn reverse_zero_cell_refusals_do_not_become_false_or_mirror_positive_relations() {
    let (mut frame, links, _) = tests::fixture();
    install(&mut frame, vec![None, cell(101), None]);
    assert_eq!(reverse_zero_shared(&frame, &links, 1), Ok(false));
    assert_eq!(
        reverse_zero_shared(&frame, &links, 2),
        Err("ReservedCountryNaming.reverse_visibility cell capture refused")
    );
    reserved_mut(&mut frame).reverse_visibility = Recorded::Value(None);
    assert_eq!(
        reverse_zero_shared(&frame, &links, 1),
        Err("ReservedCountryNaming.reverse_visibility capture refused")
    );
    reserved_mut(&mut frame).reverse_visibility = Recorded::NotRecorded;
    assert_eq!(
        reverse_zero_shared(&frame, &links, 1),
        Err("ReservedCountryNaming.reverse_visibility not recorded")
    );
    install(&mut frame, vec![None, cell(101)]);
    let stale = replay_data::rows::PacketHour::Stamped(frame.hour + 1);
    let Recorded::Value(Some(packet)) = &mut reserved_mut(&mut frame).reverse_visibility else {
        panic!()
    };
    packet.sample_hour = stale;
    assert_eq!(
        reverse_zero_shared(&frame, &links, 1),
        Err("ReservedCountryNaming.reverse_visibility sample hour mismatch")
    );
}

#[test]
fn reverse_zero_reached_target_and_subject_refusals_remain_independent() {
    let (mut frame, links, _) = tests::fixture();
    install(
        &mut frame,
        vec![
            None,
            Some(ReverseZeroVisibilityInputs {
                target: None,
                war: Some([0, 0]),
                subject: Some(subject(Some(101), Some(102))),
                military_access: None,
            }),
        ],
    );
    assert_eq!(
        reverse_zero_shared(&frame, &links, 1),
        Err("ReservedCountryNaming.reverse_visibility target capture refused")
    );
    let Recorded::Value(Some(packet)) = &mut reserved_mut(&mut frame).reverse_visibility else {
        panic!()
    };
    let cell = packet.cells[1].as_mut().unwrap();
    cell.target = Some(101);
    cell.subject = None;
    assert_eq!(
        reverse_zero_shared(&frame, &links, 1),
        Err("ReservedCountryNaming.reverse_visibility subject capture refused")
    );
    let Recorded::Value(Some(packet)) = &mut reserved_mut(&mut frame).reverse_visibility else {
        panic!()
    };
    packet.cells[1].as_mut().unwrap().subject = Some(ZeroSubjectInputs {
        relation_type: Some(engine_executable::native_values::subject_relation::TYPE_VALUE + 1),
        parties: None,
    });
    assert_eq!(reverse_zero_shared(&frame, &links, 1), Ok(false));
    assert_eq!(
        map_intel_subject(
            &ZeroSubjectInputs {
                relation_type: None,
                parties: Some([Some(0), Some(0)]),
            },
            &links,
            Ok(0)
        ),
        Err("visibility subject has inconsistent native branches")
    );
}

#[test]
fn reverse_zero_intel_uses_actual_owner_zero_and_retains_carried_sample_hour() {
    let (mut frame, mut links, _) = tests::fixture();
    frame.hour = 24;
    links.extend([(777, 0), (778, 0)]);
    install(&mut frame, vec![cell(777), cell(101), cell(102), cell(103)]);
    certificate(&mut frame, 3, 4);
    set::<tables::CountryIntel>(
        &mut frame,
        vec![
            CountryIntelRow {
                owner_country: 0,
                target_index: 0,
                army: 40_000,
                ..Default::default()
            },
            CountryIntelRow {
                owner_country: 0,
                target_index: 3,
                army: i64::MIN,
                ..Default::default()
            },
            CountryIntelRow {
                owner_country: 777,
                target_index: 0,
                army: 100_000,
                ..Default::default()
            },
            CountryIntelRow {
                owner_country: 778,
                target_index: 2,
                army: 100_000,
                ..Default::default()
            },
        ],
    );
    assert_eq!(cutoff(&frame, &links, 777, 777, CUTOFFS), Ok(-2));
    assert_eq!(cutoff(&frame, &links, 777, 102, CUTOFFS), Ok(-5));
    assert_eq!(cutoff(&frame, &links, 0, 103, CUTOFFS), Ok(5));
    assert_eq!(
        reserved(&frame)
            .unwrap()
            .country_intel_sample
            .require("sample")
            .unwrap()
            .as_ref()
            .unwrap()
            .sample_hour,
        3
    );
    let intel = Intel {
        rows: complete::<tables::CountryIntel>(&frame, "CountryIntel"),
        raws: raw_references(&links),
    };
    assert_eq!(intel.zero_army(&frame, 101, 4), Ok(0));
    assert_eq!(intel.zero_army(&frame, 101, i32::MAX as u32), Ok(0));
    assert_eq!(
        intel.zero_army(&frame, 101, i32::MAX as u32 + 1),
        Err("CountryIntel owner0 target outside native index range")
    );
}

#[test]
fn reverse_zero_intel_admission_rejects_uncertified_and_malformed_sparse_sections() {
    let (mut frame, links, _) = tests::fixture();
    install(&mut frame, vec![]);
    let check = |frame: &DecodedFrame| {
        Intel {
            rows: complete::<tables::CountryIntel>(frame, "CountryIntel"),
            raws: raw_references(&links),
        }
        .zero_army(frame, 101, 1)
    };
    assert_eq!(
        check(&frame),
        Err("ReservedCountryNaming.country_intel_sample capture refused")
    );
    reserved_mut(&mut frame).country_intel_sample = Recorded::NotRecorded;
    assert_eq!(
        check(&frame),
        Err("ReservedCountryNaming.country_intel_sample not recorded")
    );
    certificate(&mut frame, 1, 0);
    assert_eq!(
        check(&frame),
        Err("ReservedCountryNaming.country_intel_sample is later than frame")
    );
    certificate(&mut frame, 0, i32::MAX as u32 + 1);
    assert_eq!(
        check(&frame),
        Err("ReservedCountryNaming.country_intel_sample count outside native range")
    );
    certificate(&mut frame, 0, 0);
    assert_eq!(check(&frame), Ok(0));
    set::<tables::CountryIntel>(
        &mut frame,
        vec![CountryIntelRow {
            owner_country: 0,
            target_index: 0,
            ..Default::default()
        }],
    );
    assert_eq!(
        check(&frame),
        Err("CountryIntel owner0 keys outside admitted vector or duplicated")
    );
    certificate(&mut frame, 0, 2);
    assert_eq!(
        check(&frame),
        Err("CountryIntel owner0 final sentinel missing")
    );
    set::<tables::CountryIntel>(
        &mut frame,
        vec![
            CountryIntelRow {
                owner_country: 0,
                target_index: 1,
                ..Default::default()
            },
            CountryIntelRow {
                owner_country: 0,
                target_index: 1,
                ..Default::default()
            },
        ],
    );
    assert_eq!(
        check(&frame),
        Err("CountryIntel owner0 keys outside admitted vector or duplicated")
    );
    set::<tables::CountryIntel>(
        &mut frame,
        vec![CountryIntelRow {
            owner_country: 0,
            target_index: 2,
            ..Default::default()
        }],
    );
    assert_eq!(
        check(&frame),
        Err("CountryIntel owner0 keys outside admitted vector or duplicated")
    );
    *frame.tables[TableId::CountryIntel.index()] = TableView::Truncated {
        rows: TableRows::CountryIntel(vec![CountryIntelRow {
            owner_country: 0,
            target_index: 1,
            ..Default::default()
        }]),
        missing: 1,
    };
    assert_eq!(check(&frame), Err("CountryIntel"));
}

#[test]
fn reverse_zero_signed_getter_gate_needs_no_certificate_but_disclosure_still_needs_links() {
    let (mut frame, mut links, _) = tests::fixture();
    let intel = Intel {
        rows: Err("CountryIntel"),
        raws: BTreeMap::new(),
    };
    for raw in [0, i32::MIN as u32, u32::MAX] {
        assert_eq!(intel.zero_army(&frame, raw, 1), Ok(0));
    }
    assert_eq!(
        cutoff(&frame, &links, 0, u32::MAX, CUTOFFS),
        Err("visibility country link")
    );
    links.insert(u32::MAX, 1);
    assert_eq!(
        cutoff(&frame, &links, 0, u32::MAX, CUTOFFS),
        Err("ReservedCountryNaming")
    );
    install(&mut frame, vec![None, cell(u32::MAX)]);
    *frame.tables[TableId::CountryIntel.index()] = TableView::Unavailable {
        reason: "signed raw gate skips getter vector".into(),
    };
    assert_eq!(cutoff(&frame, &links, 0, u32::MAX, CUTOFFS), Ok(-5));
}

#[test]
fn reverse_zero_optional_failures_preserve_positive_intel_and_forward_zero_refusals() {
    let (mut frame, mut links, _) = tests::fixture();
    links.insert(777, 0);
    install(&mut frame, vec![cell(0), cell(101)]);
    reserved_mut(&mut frame).faction = Recorded::Value(Some(7));
    set::<tables::FactionMembers>(
        &mut frame,
        vec![FactionMemberRow {
            country: 101,
            faction: 7,
            ordinal: 0,
        }],
    );
    set::<tables::CountryIntel>(
        &mut frame,
        vec![CountryIntelRow {
            owner_country: 101,
            target_index: 2,
            army: 40_000,
            ..Default::default()
        }],
    );
    assert_eq!(cutoff(&frame, &links, 101, 102, CUTOFFS), Ok(-2));
    set::<tables::CountryCounterInputs>(
        &mut frame,
        vec![CountryCounterInputRow {
            country: 101,
            zero_war: Recorded::Value(None),
            forward_visibility: Recorded::NotRecorded,
            ..Default::default()
        }],
    );
    assert_eq!(
        cutoff(&frame, &links, 101, 777, CUTOFFS),
        Err("CountryCounterInputs.zero_war capture refused")
    );
    assert_eq!(
        cutoff(&frame, &links, 101, 0, CUTOFFS),
        Err("CountryCounterInputs.zero_war capture refused")
    );
    assert_eq!(cutoff(&frame, &links, 101, 102, CUTOFFS), Ok(-2));
}

#[test]
fn reverse_zero_neighbour_stamp_preserves_raw_zero_against_positive_alias_zero() {
    let (mut frame, mut links, content) = tests::fixture();
    links.insert(777, 0);
    install(&mut frame, vec![cell(777)]);
    certificate(&mut frame, 0, 1);
    set::<tables::CountryIntel>(
        &mut frame,
        vec![CountryIntelRow {
            owner_country: 0,
            target_index: 0,
            army: 40_000,
            ..Default::default()
        }],
    );
    let relations = Relations::default();
    let mut rebuild = Rebuild {
        frame: &frame,
        links: &links,
        topology: &content.topology,
        controllers: [(1, 0), (2, 777)].into(),
        intel: Intel {
            rows: complete::<tables::CountryIntel>(&frame, "CountryIntel"),
            raws: raw_references(&links),
        },
        relations: &relations,
        maximum: 100_000,
        cutoffs: CUTOFFS,
        bytes: [(1, 0), (2, 0)].into(),
    };
    rebuild.neighbors(1, 150).unwrap();
    assert_eq!(rebuild.bytes[&2], 148);
    rebuild.neighbors(2, 150).unwrap();
    assert_eq!(rebuild.bytes[&1], 155);
    assert_eq!(map_intel_equal(&links, 0, 777), Ok(false));
    assert_eq!(map_intel_equal(&links, 777, 777), Ok(true));
}

#[test]
fn event_sharing_compares_receiver_object_id_before_its_incoming_alias() {
    let (mut frame, mut links, _) = tests::fixture();
    links.extend([(777, 0), (778, 0), (701, 1)]);
    assert_eq!(event_shared(&frame, &links, 777, 0, true), Ok(true));
    assert_eq!(
        event_shared(&frame, &links, 777, 777, false),
        Err("ReservedCountryNaming")
    );
    assert_eq!(event_shared(&frame, &links, 701, 101, true), Ok(true));
    install(
        &mut frame,
        vec![Some(ReverseZeroVisibilityInputs {
            war: Some([1, 0]),
            ..Default::default()
        })],
    );
    assert_eq!(event_shared(&frame, &links, 777, 777, true), Ok(false));
    assert_eq!(event_shared(&frame, &links, 777, 778, false), Ok(false));
    assert_eq!(event_shared(&frame, &links, 777, 0, true), Ok(true));
}

#[test]
fn event_reverse_military_access_is_directed_lazy_and_war_vetoed() {
    let (mut frame, links, _) = tests::fixture();
    install(
        &mut frame,
        vec![
            None,
            Some(ReverseZeroVisibilityInputs {
                war: Some([0, 0]),
                military_access: Some(1),
                ..Default::default()
            }),
        ],
    );
    reserved_mut(&mut frame).faction = Recorded::NotRecorded;
    assert_eq!(event_shared(&frame, &links, 0, 101, true), Ok(true));
    assert_eq!(
        event_shared(&frame, &links, 0, 101, false),
        Err("ReservedCountryNaming.faction not recorded")
    );
    let Recorded::Value(Some(packet)) = &mut reserved_mut(&mut frame).reverse_visibility else {
        panic!()
    };
    packet.cells[1].as_mut().unwrap().war = Some([1, 0]);
    packet.cells[1].as_mut().unwrap().military_access = None;
    assert_eq!(event_shared(&frame, &links, 0, 101, true), Ok(false));
}

#[test]
fn event_reverse_unknown_military_access_is_not_absence_and_flag_zero_skips_it() {
    let (mut frame, links, _) = tests::fixture();
    for military_access in [None, Some(0), Some(2)] {
        let mut operands = cell(101).unwrap();
        operands.military_access = military_access;
        install(&mut frame, vec![None, Some(operands)]);
        assert_eq!(event_shared(&frame, &links, 0, 101, false), Ok(false));
        match military_access {
            Some(0) => assert_eq!(event_shared(&frame, &links, 0, 101, true), Ok(false)),
            None => assert_eq!(
                event_shared(&frame, &links, 0, 101, true),
                Err("ReservedCountryNaming.reverse_visibility military access capture refused")
            ),
            Some(_) => assert_eq!(
                event_shared(&frame, &links, 0, 101, true),
                Err("ReservedCountryNaming.reverse_visibility military access invalid")
            ),
        }
    }
}

#[test]
fn event_positive_military_access_has_direction_and_precedes_faction_or_subject() {
    let (mut frame, links, _) = tests::fixture();
    set::<tables::RelationLinks>(
        &mut frame,
        vec![RelationLinkRow {
            country: 101,
            target_index: 2,
            links: RelationLinkRow::MILITARY_ACCESS,
            transport_subject: Default::default(),
        }],
    );
    *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "not reached".into(),
    };
    *frame.tables[TableId::Subjects.index()] = TableView::Unavailable {
        reason: "not reached".into(),
    };
    assert_eq!(event_shared(&frame, &links, 101, 102, true), Ok(true));
    assert_eq!(
        event_shared(&frame, &links, 102, 101, true),
        Err("FactionMembers")
    );
    assert_eq!(
        event_shared(&frame, &links, 101, 102, false),
        Err("FactionMembers")
    );
    set::<tables::WarRelations>(
        &mut frame,
        vec![replay_data::rows::WarRelationRow {
            first_country: 101,
            second_country: 102,
            ..Default::default()
        }],
    );
    assert_eq!(event_shared(&frame, &links, 101, 102, true), Ok(false));
}

#[test]
fn event_positive_status_zero_does_not_infer_military_absence_from_relation_links() {
    let (mut frame, mut links, _) = tests::fixture();
    links.insert(777, 0);
    set::<tables::CountryCounterInputs>(
        &mut frame,
        vec![CountryCounterInputRow {
            country: 101,
            zero_war: Recorded::Value(Some([0, 0])),
            forward_visibility: Recorded::NotRecorded,
            zero_subject: Recorded::Value(Some(ZeroSubjectInputs::default())),
            ..Default::default()
        }],
    );
    assert_eq!(
        event_shared(&frame, &links, 101, 0, true),
        Err("CountryCounterInputs.forward_visibility not recorded")
    );
    assert_eq!(
        event_shared(&frame, &links, 101, 777, true),
        Err("CountryCounterInputs.forward_visibility not recorded")
    );
    set::<tables::CountryCounterInputs>(
        &mut frame,
        vec![CountryCounterInputRow {
            country: 101,
            zero_war: Recorded::Value(Some([1, 0])),
            forward_visibility: Recorded::Value(Some(ReverseZeroVisibilityInputs {
                war: Some([1, 0]),
                ..Default::default()
            })),
            ..Default::default()
        }],
    );
    assert_eq!(event_shared(&frame, &links, 101, 777, true), Ok(false));
}

#[test]
fn event_reverse_independent_faction_or_subject_positive_survives_unknown_military() {
    let (mut frame, links, _) = tests::fixture();
    install(&mut frame, vec![None, cell(101)]);
    reserved_mut(&mut frame).faction = Recorded::Value(Some(7));
    set::<tables::FactionMembers>(
        &mut frame,
        vec![FactionMemberRow {
            country: 101,
            faction: 7,
            ordinal: 0,
        }],
    );
    assert_eq!(event_shared(&frame, &links, 0, 101, true), Ok(true));
    reserved_mut(&mut frame).faction = Recorded::NotRecorded;
    let Recorded::Value(Some(packet)) = &mut reserved_mut(&mut frame).reverse_visibility else {
        panic!()
    };
    packet.cells[1].as_mut().unwrap().subject = Some(subject(None, Some(101)));
    assert_eq!(event_shared(&frame, &links, 0, 101, true), Ok(true));
    assert_eq!(event_shared(&frame, &links, 0, 101, false), Ok(true));
    let Recorded::Value(Some(packet)) = &mut reserved_mut(&mut frame).reverse_visibility else {
        panic!()
    };
    packet.cells[1].as_mut().unwrap().war = None;
    assert_eq!(
        event_shared(&frame, &links, 0, 101, true),
        Err("ReservedCountryNaming.reverse_visibility war capture refused")
    );
}

#[test]
fn event_forward_status_zero_known_subject_survives_unknown_faction_and_military() {
    let (mut frame, links, _) = tests::fixture();
    *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "unknown".into(),
    };
    set::<tables::CountryCounterInputs>(
        &mut frame,
        vec![CountryCounterInputRow {
            country: 101,
            zero_war: Recorded::Value(None),
            zero_subject: Recorded::Value(None),
            forward_visibility: Recorded::Value(Some(ReverseZeroVisibilityInputs {
                target: Some(0),
                war: Some([0, 0]),
                subject: Some(subject(None, Some(0))),
                military_access: None,
            })),
            ..Default::default()
        }],
    );
    assert_eq!(event_shared(&frame, &links, 101, 0, true), Ok(true));
    assert_eq!(event_shared(&frame, &links, 101, 0, false), Ok(true));
    set::<tables::CountryCounterInputs>(
        &mut frame,
        vec![CountryCounterInputRow {
            country: 101,
            zero_war: Recorded::Value(None),
            zero_subject: Recorded::Value(Some(subject(None, Some(0)))),
            forward_visibility: Recorded::Value(Some(ReverseZeroVisibilityInputs {
                target: Some(0),
                war: None,
                subject: Some(subject(None, Some(0))),
                military_access: Some(1),
            })),
            ..Default::default()
        }],
    );
    assert_eq!(
        event_shared(&frame, &links, 101, 0, true),
        Err("CountryCounterInputs.forward_visibility war capture refused")
    );
}

#[test]
fn event_positive_subject_truth_survives_unknown_access_and_faction_after_known_war() {
    let (mut frame, links, _) = tests::fixture();
    *frame.tables[TableId::RelationLinks.index()] = TableView::Unavailable {
        reason: "unknown".into(),
    };
    *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "unknown".into(),
    };
    set::<tables::Subjects>(
        &mut frame,
        vec![replay_data::rows::SubjectRow {
            overlord: 101,
            subject: 102,
            ..Default::default()
        }],
    );
    assert_eq!(event_shared(&frame, &links, 101, 102, true), Ok(true));
    set::<tables::Subjects>(&mut frame, vec![]);
    assert_eq!(
        event_shared(&frame, &links, 101, 102, true),
        Err("RelationLinks")
    );
    *frame.tables[TableId::WarRelations.index()] = TableView::Unavailable {
        reason: "unknown".into(),
    };
    assert_eq!(
        event_shared(&frame, &links, 101, 102, true),
        Err("WarRelations")
    );
}

fn forward_packet(frame: &mut DecodedFrame, packet: Recorded<Option<ReverseZeroVisibilityInputs>>) {
    set::<tables::CountryCounterInputs>(
        frame,
        vec![CountryCounterInputRow {
            country: 101,
            forward_visibility: packet,
            zero_war: Recorded::Value(Some([0, 0])),
            zero_subject: Recorded::Value(Some(ZeroSubjectInputs::default())),
            ..Default::default()
        }],
    );
}

#[test]
fn forward_visibility_war_and_military_precede_unknown_cell_target_and_faction() {
    let (mut frame, mut links, _) = tests::fixture();
    links.insert(777, 0);
    *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "not reached".into(),
    };
    for (war, access, expected) in [
        (Some([1, 0]), Some(2), Ok(false)),
        (Some([0, 0]), Some(1), Ok(true)),
        (
            None,
            Some(1),
            Err("CountryCounterInputs.forward_visibility war capture refused"),
        ),
    ] {
        forward_packet(
            &mut frame,
            Recorded::Value(Some(ReverseZeroVisibilityInputs {
                war,
                military_access: access,
                ..Default::default()
            })),
        );
        assert_eq!(event_shared(&frame, &links, 101, 777, true), expected);
    }
}

#[test]
fn forward_visibility_fog_uses_raw_cell_alias_and_only_historical_absence_uses_legacy() {
    let (mut frame, mut links, _) = tests::fixture();
    links.extend([(777, 0), (778, 0)]);
    forward_packet(&mut frame, Recorded::NotRecorded);
    assert_eq!(forward_zero_shared(&frame, &links, 1), Ok(false));
    assert_eq!(
        event_shared(&frame, &links, 101, 777, true),
        Err("CountryCounterInputs.forward_visibility not recorded")
    );
    forward_packet(&mut frame, Recorded::Value(None));
    assert_eq!(
        forward_zero_shared(&frame, &links, 1),
        Err("CountryCounterInputs.forward_visibility capture refused")
    );
    forward_packet(
        &mut frame,
        Recorded::Value(Some(ReverseZeroVisibilityInputs {
            target: Some(777),
            war: Some([0, 0]),
            subject: Some(subject(None, Some(778))),
            military_access: Some(2),
        })),
    );
    assert_eq!(forward_zero_shared(&frame, &links, 1), Ok(true));
    assert_eq!(event_shared(&frame, &links, 101, 0, false), Ok(true));
    assert_eq!(
        event_shared(&frame, &links, 101, 777, true),
        Err("CountryCounterInputs.forward_visibility military access invalid")
    );
    forward_packet(
        &mut frame,
        Recorded::Value(Some(ReverseZeroVisibilityInputs {
            target: Some(0),
            war: Some([0, 0]),
            subject: Some(subject(Some(777), Some(778))),
            military_access: Some(0),
        })),
    );
    assert_eq!(forward_zero_shared(&frame, &links, 1), Ok(false));
    assert_eq!(event_shared(&frame, &links, 101, 777, true), Ok(false));
}

#[test]
fn forward_visibility_known_faction_or_subject_survives_unknown_military_but_not_malformed() {
    let (mut frame, links, _) = tests::fixture();
    install(&mut frame, vec![]);
    reserved_mut(&mut frame).faction = Recorded::Value(Some(7));
    set::<tables::FactionMembers>(
        &mut frame,
        vec![FactionMemberRow {
            country: 101,
            faction: 7,
            ordinal: 0,
        }],
    );
    forward_packet(
        &mut frame,
        Recorded::Value(Some(ReverseZeroVisibilityInputs {
            war: Some([0, 0]),
            ..Default::default()
        })),
    );
    assert_eq!(event_shared(&frame, &links, 101, 0, true), Ok(true));
    forward_packet(
        &mut frame,
        Recorded::Value(Some(ReverseZeroVisibilityInputs {
            war: Some([0, 0]),
            military_access: Some(2),
            ..Default::default()
        })),
    );
    assert_eq!(
        event_shared(&frame, &links, 101, 0, true),
        Err("CountryCounterInputs.forward_visibility military access invalid")
    );
    *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "unknown".into(),
    };
    forward_packet(
        &mut frame,
        Recorded::Value(Some(ReverseZeroVisibilityInputs {
            target: Some(0),
            war: Some([0, 0]),
            subject: Some(subject(None, Some(0))),
            military_access: None,
        })),
    );
    assert_eq!(event_shared(&frame, &links, 101, 0, true), Ok(true));
    forward_packet(
        &mut frame,
        Recorded::Value(Some(ReverseZeroVisibilityInputs {
            target: Some(0),
            war: Some([0, 0]),
            subject: Some(ZeroSubjectInputs::default()),
            military_access: None,
        })),
    );
    assert_eq!(
        event_shared(&frame, &links, 101, 0, true),
        Err("CountryCounterInputs.forward_visibility military access capture refused")
    );
}

#[test]
fn forward_visibility_packet_refusal_and_nested_failures_never_backfill_legacy() {
    let (mut frame, links, _) = tests::fixture();
    for (packet, expected) in [
        (
            Recorded::NotRecorded,
            Err("CountryCounterInputs.forward_visibility not recorded"),
        ),
        (
            Recorded::Value(None),
            Err("CountryCounterInputs.forward_visibility capture refused"),
        ),
        (
            Recorded::Value(Some(ReverseZeroVisibilityInputs::default())),
            Err("CountryCounterInputs.forward_visibility war capture refused"),
        ),
    ] {
        forward_packet(&mut frame, packet);
        assert_eq!(event_shared(&frame, &links, 101, 0, true), expected);
    }
    for (target, subject, expected) in [
        (
            None,
            Some(subject(Some(0), Some(0))),
            Err("CountryCounterInputs.forward_visibility target capture refused"),
        ),
        (
            Some(0),
            None,
            Err("CountryCounterInputs.forward_visibility subject capture refused"),
        ),
    ] {
        forward_packet(
            &mut frame,
            Recorded::Value(Some(ReverseZeroVisibilityInputs {
                target,
                war: Some([0, 0]),
                subject,
                military_access: Some(0),
            })),
        );
        assert_eq!(event_shared(&frame, &links, 101, 0, true), expected);
    }
}

#[test]
fn forward_visibility_null_and_wrong_type_subjects_do_not_demand_cell_target() {
    let (mut frame, links, _) = tests::fixture();
    for subject in [
        ZeroSubjectInputs::default(),
        ZeroSubjectInputs {
            relation_type: Some(engine_executable::native_values::subject_relation::TYPE_VALUE.wrapping_add(1)),
            parties: None,
        },
    ] {
        forward_packet(
            &mut frame,
            Recorded::Value(Some(ReverseZeroVisibilityInputs {
                target: None,
                war: Some([0, 0]),
                subject: Some(subject),
                military_access: Some(0),
            })),
        );
        assert_eq!(event_shared(&frame, &links, 101, 0, true), Ok(false));
        assert_eq!(forward_zero_shared(&frame, &links, 1), Ok(false));
    }
    forward_packet(
        &mut frame,
        Recorded::Value(Some(ReverseZeroVisibilityInputs {
            target: None,
            war: Some([0, 0]),
            subject: Some(ZeroSubjectInputs {
                relation_type: Some(engine_executable::native_values::subject_relation::TYPE_VALUE),
                parties: None,
            }),
            military_access: Some(0),
        })),
    );
    assert_eq!(
        event_shared(&frame, &links, 101, 0, true),
        Err("visibility subject has inconsistent native branches")
    );
}

#[test]
fn reverse_zero_null_and_wrong_type_subjects_do_not_demand_cell_target() {
    let (mut frame, links, _) = tests::fixture();
    for subject in [
        ZeroSubjectInputs::default(),
        ZeroSubjectInputs {
            relation_type: Some(engine_executable::native_values::subject_relation::TYPE_VALUE.wrapping_add(1)),
            parties: None,
        },
    ] {
        install(
            &mut frame,
            vec![
                None,
                Some(ReverseZeroVisibilityInputs {
                    target: None,
                    war: Some([0, 0]),
                    subject: Some(subject),
                    military_access: Some(0),
                }),
            ],
        );
        assert_eq!(reverse_zero_shared(&frame, &links, 1), Ok(false));
        assert_eq!(event_shared(&frame, &links, 0, 101, true), Ok(false));
        assert_eq!(event_shared(&frame, &links, 0, 101, false), Ok(false));
    }
    for (subject, expected) in [
        (
            None,
            Err("ReservedCountryNaming.reverse_visibility subject capture refused"),
        ),
        (
            Some(subject(Some(101), Some(101))),
            Err("ReservedCountryNaming.reverse_visibility target capture refused"),
        ),
        (
            Some(ZeroSubjectInputs {
                relation_type: None,
                parties: Some([Some(0), Some(0)]),
            }),
            Err("visibility subject has inconsistent native branches"),
        ),
    ] {
        install(
            &mut frame,
            vec![
                None,
                Some(ReverseZeroVisibilityInputs {
                    target: None,
                    war: Some([0, 0]),
                    subject,
                    military_access: Some(0),
                }),
            ],
        );
        assert_eq!(reverse_zero_shared(&frame, &links, 1), expected);
        assert_eq!(event_shared(&frame, &links, 0, 101, true), expected);
    }
    let Recorded::Value(Some(packet)) = &mut reserved_mut(&mut frame).reverse_visibility else {
        panic!()
    };
    packet.cells[1].as_mut().unwrap().war = None;
    assert_eq!(
        event_shared(&frame, &links, 0, 101, true),
        Err("ReservedCountryNaming.reverse_visibility war capture refused")
    );
}

#[test]
fn forward_visibility_complete_current_counter_table_is_required() {
    let (mut frame, links, _) = tests::fixture();
    let rows = TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
        country: 101,
        forward_visibility: Recorded::Value(Some(ReverseZeroVisibilityInputs {
            war: Some([0, 0]),
            military_access: Some(1),
            ..Default::default()
        })),
        ..Default::default()
    }]);
    for view in [
        TableView::Truncated { rows, missing: 1 },
        TableView::Unavailable {
            reason: "hour not sampled".into(),
        },
    ] {
        *frame.tables[TableId::CountryCounterInputs.index()] = view;
        assert_eq!(
            event_shared(&frame, &links, 101, 0, true),
            Err("CountryCounterInputs")
        );
    }
    set::<tables::CountryCounterInputs>(&mut frame, vec![]);
    assert_eq!(
        event_shared(&frame, &links, 101, 0, true),
        Err("CountryCounterInputs visibility owner missing")
    );
    let row = CountryCounterInputRow {
        country: 101,
        forward_visibility: Recorded::NotRecorded,
        ..Default::default()
    };
    set::<tables::CountryCounterInputs>(&mut frame, vec![row.clone(), row]);
    assert_eq!(
        event_shared(&frame, &links, 101, 0, true),
        Err("CountryCounterInputs visibility owner duplicated")
    );
}

#[test]
fn forward_visibility_does_not_change_positive_pairs_or_actual_zero_self_comparison() {
    let (mut frame, mut links, _) = tests::fixture();
    links.insert(777, 0);
    *frame.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
        reason: "not reached".into(),
    };
    assert_eq!(event_shared(&frame, &links, 101, 102, false), Ok(false));
    assert_eq!(event_shared(&frame, &links, 777, 0, true), Ok(true));
    install(
        &mut frame,
        vec![
            None,
            Some(ReverseZeroVisibilityInputs {
                war: Some([0, 0]),
                military_access: Some(1),
                ..Default::default()
            }),
        ],
    );
    assert_eq!(event_shared(&frame, &links, 0, 101, true), Ok(true));
}

#[test]
fn forward_visibility_reverse_reached_malformed_military_cannot_be_or_dominated() {
    let (mut frame, links, _) = tests::fixture();
    install(&mut frame, vec![None, cell(101)]);
    reserved_mut(&mut frame).faction = Recorded::Value(Some(7));
    set::<tables::FactionMembers>(
        &mut frame,
        vec![FactionMemberRow {
            country: 101,
            faction: 7,
            ordinal: 0,
        }],
    );
    for (war, access, flag, expected) in [
        (
            Some([0, 0]),
            Some(2),
            true,
            Err("ReservedCountryNaming.reverse_visibility military access invalid"),
        ),
        (Some([1, 0]), Some(2), true, Ok(false)),
        (Some([0, 0]), Some(2), false, Ok(true)),
        (Some([0, 0]), None, true, Ok(true)),
    ] {
        let Recorded::Value(Some(packet)) = &mut reserved_mut(&mut frame).reverse_visibility else {
            panic!()
        };
        packet.cells[1] = Some(ReverseZeroVisibilityInputs {
            war,
            military_access: access,
            ..Default::default()
        });
        assert_eq!(event_shared(&frame, &links, 0, 101, flag), expected);
    }
}
