use replay_data::{
    Hour, rows::CountryLinkRow, rows::FactionMemberRow, rows::TableId, rows::TableRows,
    rows::WarRelationRow, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::TableView};
use std::collections::BTreeMap;

pub(crate) const NIGHT_HOUR: Hour = 60_826_315;

pub(crate) const NO_END_HOUR: i32 = 43_808_760;

pub(crate) fn frame(wars: Vec<WarRelationRow>, members: Vec<FactionMemberRow>) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not needed for the allegiance tables".into(),
    });
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (0..3)
            .map(|slot| CountryLinkRow {
                id: 100 + slot,
                canonical_slot: slot,
            })
            .collect(),
    ));
    tables[TableId::LocalPlayer.index()] = TableView::Complete(TableRows::LocalPlayer(vec![
        replay_data::rows::LocalPlayerRow {
            country: 100,
            observer_country: 0,
        },
    ]));
    tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(wars));
    tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(members));
    DecodedFrame {
        branch: 2,
        hour: NIGHT_HOUR,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}

pub fn links(frame: &DecodedFrame) -> BTreeMap<u32, u32> {
    frame
        .complete::<tables::CountryLinks>()
        .unwrap()
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect()
}

pub(crate) fn war(first: u32, second: u32, start_hour: i32, end_hour: i32) -> WarRelationRow {
    WarRelationRow {
        first_country: first,
        second_country: second,
        start_hour,
        end_hour,
        ..Default::default()
    }
}

pub(crate) fn member(country: u32, faction: u64) -> FactionMemberRow {
    FactionMemberRow {
        country,
        faction,
        ordinal: 0,
    }
}

pub fn native_frame(rows: Vec<replay_data::rows::CountryCounterInputRow>) -> DecodedFrame {
    let mut frame = frame(
        vec![war(101, 102, 60_759_371, NO_END_HOUR)],
        vec![member(101, 7), member(102, 7)],
    );
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(rows));
    frame
}

pub fn native_row(
    country: u32,
    targets: Vec<u32>,
    zero_war: [u8; 2],
) -> replay_data::rows::CountryCounterInputRow {
    replay_data::rows::CountryCounterInputRow {
        forward_visibility: replay_data::rows::Recorded::NotRecorded,
        country,
        co_belligerents: Some(targets).into(),
        zero_war: Some(zero_war).into(),
        ..Default::default()
    }
}
