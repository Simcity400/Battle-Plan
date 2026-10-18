//! Peace conference frames and replays for tests here and in the crates above.

use super::*;
use replay::spool::Record;
use replay_data::{
    identity::Identity, identity::Roster, identity::SampleBoundary, identity::SessionStart,
    identity::Verification, rows::*,
};
use replay_frame::{
    branches::BranchReason, branches::BranchStart, branches::Parentage, frame::FrameEncoder,
    frame::Metrics, frame::TableView,
};

pub const CAMPAIGN: &[u8] = b"derived-peace-fixture";

pub fn frame(hour: i32, count: u32, settled: bool, conferences: &[u64]) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "outside peace fixture".into(),
    });
    tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: CAMPAIGN.into(),
        }]));
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|id| CountryRow {
                id: 10_000 + id,
                canonical_slot: id,
                tag: format!("country/{id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [1_000, 10_000]
            .into_iter()
            .flat_map(|base| {
                (1..=count).map(move |id| CountryLinkRow {
                    id: base + id,
                    canonical_slot: id,
                })
            })
            .collect(),
    ));
    tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 77,
            literal: b"take_states".to_vec(),
        }]));
    tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        (1..=count)
            .map(|id| StateRow {
                id,
                owner: 1_000
                    + if settled && id % 2 == 1 && id < count {
                        id + 1
                    } else {
                        id
                    },
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        (1..=count)
            .flat_map(|id| {
                [
                    ProvinceRow {
                        id: id * 2,
                        state: id,
                        victory_points: 7,
                        ..Default::default()
                    },
                    ProvinceRow {
                        id: id * 2 + 1,
                        state: id,
                        victory_points: -2,
                        ..Default::default()
                    },
                ]
            })
            .collect(),
    ));
    tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(
        (1..count)
            .step_by(2)
            .map(|id| WarRelationRow {
                first_country: 1_000 + id,
                second_country: 1_001 + id,
                start_hour: 80,
                end_hour: if settled { 101 } else { 0 },
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::PeaceConferences.index()] = TableView::Complete(TableRows::PeaceConferences(
        conferences
            .iter()
            .enumerate()
            .map(|(ordinal, &id)| PeaceConferenceRow {
                id,
                ordinal: ordinal as u32,
                current_turn: hour - 100,
                finalized: 0,
            })
            .collect(),
    ));
    tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(vec![]));
    tables[TableId::PeaceSettlements.index()] =
        TableView::Complete(TableRows::PeaceSettlements(vec![]));
    tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            (1..=count)
                .map(|id| CountryCounterInputRow {
                    country: id + 1_000,
                    owned_state_count: if settled && id % 2 == 1 && id < count {
                        0
                    } else {
                        1
                    },
                    ..Default::default()
                })
                .collect(),
        ));
    tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(
        (1..=count)
            .map(|id| PoliticsRow {
                country: id + 1_000,
                ruling_party: Some(3),
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::PoliticalParties.index()] = TableView::Complete(TableRows::PoliticalParties(
        (1..=count)
            .map(|id| PoliticalPartyRow {
                country: id + 1_000,
                id: 3,
                token: b"authored_group".to_vec(),
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::CountryAppearance.index()] = TableView::Complete(TableRows::CountryAppearance(
        (1..=count)
            .map(|id| CountryAppearanceRow {
                country: id + 1_000,
                leader_ideology: b"authored_ideology".to_vec(),
                ..Default::default()
            })
            .collect(),
    ));
    let mut metrics = Metrics::default();
    metrics.table_ns[TableId::PeaceConferences.index()] = 1;
    DecodedFrame {
        branch: 1,
        hour,
        tables: tables.map(Into::into),
        metrics,
    }
}

pub fn replay(frames: Vec<DecodedFrame>) -> Replay {
    let first = frames[0].hour;
    let identity = Identity {
        game_version: "synthetic".into(),
        exe_hash: [1; 32],
        exe_size: 1,
        vanilla_hash: [2; 32],
        dlc: vec![],
        mods: vec![],
        coverage_policy: 1,
        language: "l_english".into(),
        recorder_version: "test".into(),
        collector_hash: [3; 32],
        verification: Verification::Verified {
            launcher: Roster::default(),
            memory: Roster::default(),
        },
    };
    let mut records = vec![
        Record::SessionStart(SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 1,
            sample_boundary: SampleBoundary::EndOfHour,
            identity,
        }),
        Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            first_hour: first,
            loaded_hour: None,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    records.extend(
        frames
            .into_iter()
            .map(|frame| Record::Frame(encoder.encode(frame, false).unwrap())),
    );
    Replay::from_records(CAMPAIGN.into(), &[records]).unwrap()
}
