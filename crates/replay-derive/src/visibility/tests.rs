use super::*;
use crate::{
    frame::{Metrics, TableView},
    rows::{self, TableId, TableRows},
};
fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 22,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "fixture input unavailable".into(),
            }
            .into()
        }),
    };
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        rows::CountryRow {
            id: 101,
            canonical_slot: 1,
            ..Default::default()
        },
        rows::CountryRow {
            id: 202,
            canonical_slot: 2,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![]));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![]));
    *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(vec![]));
    frame
}
#[test]
fn design_reveal_keeps_full_thresholds_fog_bypass_and_each_intel_branch() {
    let mut frame = frame();
    let links = BTreeMap::from([(101, 1), (202, 2)]);
    let query = |frame: &DecodedFrame, target, fog, bypass, branch, maximum, threshold| {
        design_details(
            frame, &links, 101, target, fog, bypass, branch, maximum, threshold,
        )
    };
    assert_eq!(
        query(
            &frame,
            101,
            Err("fog unavailable"),
            false,
            IntelBranch::Army,
            100_000,
            100_000
        ),
        Ok(true)
    );
    assert_eq!(
        query(
            &frame,
            202,
            Ok(false),
            false,
            IntelBranch::Army,
            100_000,
            100_001
        ),
        Ok(false)
    );
    assert_eq!(
        query(
            &frame,
            u32::MAX,
            Err("fog unavailable"),
            true,
            IntelBranch::Army,
            100_000,
            99_999
        ),
        Ok(true)
    );
    assert_eq!(
        query(
            &frame,
            202,
            Err("fog unavailable"),
            false,
            IntelBranch::Army,
            100_000,
            1
        ),
        Err("fog unavailable")
    );
    *frame.tables[TableId::CountryIntel.index()] =
        TableView::Complete(TableRows::CountryIntel(vec![CountryIntelRow {
            owner_country: 101,
            target_index: 2,
            army: 30_000,
            navy: 60_000,
            air: 90_000,
            ..Default::default()
        }]));
    assert_eq!(
        query(
            &frame,
            202,
            Ok(true),
            false,
            IntelBranch::Army,
            100_000,
            30_001
        ),
        Ok(false)
    );
    assert_eq!(
        query(
            &frame,
            202,
            Ok(true),
            false,
            IntelBranch::Navy,
            100_000,
            60_000
        ),
        Ok(true)
    );
    assert_eq!(
        query(
            &frame,
            202,
            Ok(true),
            false,
            IntelBranch::Air,
            100_000,
            90_000
        ),
        Ok(true)
    );
    assert_eq!(
        query(
            &frame,
            202,
            Ok(true),
            false,
            IntelBranch::Army,
            0,
            i64::from(u32::MAX)
        ),
        Ok(true)
    );
    assert_eq!(
        intel_fraction(i64::MIN + 1, 100_000),
        Ok(i64::from(u32::MAX))
    );
    assert_eq!(intel_fraction(12_347, 37_001), Ok(33_369));
}
#[test]
fn design_reveal_actual_zero_owner_requires_its_own_vector_certificate() {
    let mut frame = frame();
    let links = BTreeMap::from([(202, 2)]);
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            rows::ReservedCountryNamingRow {
                faction: Recorded::Value(Some(0)),
                reverse_visibility: Recorded::Value(Some(rows::ReservedReverseVisibilityInputs {
                    sample_hour: rows::PacketHour::Stamped(22),
                    cells: vec![
                        None,
                        None,
                        Some(rows::ReverseZeroVisibilityInputs {
                            target: Some(202),
                            war: Some([0, 0]),
                            subject: Some(ZeroSubjectInputs::default()),
                            ..Default::default()
                        }),
                    ],
                })),
                ..Default::default()
            },
        ]));
    let query = |frame: &DecodedFrame| {
        design_details(
            frame,
            &links,
            0,
            202,
            Ok(true),
            false,
            IntelBranch::Air,
            100_000,
            90_000,
        )
    };
    assert_eq!(
        query(&frame),
        Err("ReservedCountryNaming.country_intel_sample capture refused")
    );
    let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    else {
        unreachable!()
    };
    rows[0].country_intel_sample = Recorded::Value(Some(rows::ReservedCountryIntelSample {
        sample_hour: 0,
        vector_count: 3,
    }));
    *frame.tables[TableId::CountryIntel.index()] =
        TableView::Complete(TableRows::CountryIntel(vec![CountryIntelRow {
            owner_country: 0,
            target_index: 2,
            air: 90_000,
            ..Default::default()
        }]));
    assert_eq!(query(&frame), Ok(true));
}
