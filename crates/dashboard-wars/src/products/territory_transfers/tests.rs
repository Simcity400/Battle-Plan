use super::*;
use crate::products::peace::{PeaceCaptured, PeaceDates, PeaceObservation, PeaceTransferGraph, PeaceTransferLink, PeaceTransferredState, PeaceUnobserved};

fn country(slot: u32, tag: &str, hour: i32) -> PeaceTransferCountry {
    PeaceTransferCountry { slot, country_id: slot + 100, tag: tag.to_owned(), hour }
}

fn state(state: u32, from: PeaceTransferCountry, to: PeaceTransferCountry, after: i32, points: Option<&str>, read: Option<i32>) -> PeaceTransferredState {
    PeaceTransferredState {
        state,
        from_country: from,
        to_country: to,
        dates: PeaceDates { before_hour: after - 1, after_hour: after },
        victory_points: points.map(str::to_owned),
        victory_points_read_hour: read,
        released: false,
    }
}

fn deal(id: &str, captured: Option<(i32, i32)>, states: Vec<PeaceTransferredState>) -> PeaceDeal {
    PeaceDeal {
        id: id.to_owned(),
        derived: captured.is_none(),
        observation: PeaceObservation::RecordedConference,
        conference_id: None,
        war_name: None,
        first_observed_hour: 0,
        last_observed_hour: 0,
        current_turn: None,
        finalized: None,
        first_finalized_hour: None,
        end_hour: None,
        end_is_by: false,
        first_absent_hour: None,
        participants_derived: false,
        winners: Vec::new(),
        losers: Vec::new(),
        items: Vec::new(),
        captured: captured.map(|(hour, publication_hour)| PeaceCaptured {
            hour,
            publication_hour,
            winners: Vec::new(),
            losers: Vec::new(),
            actions: Vec::new(),
            transfers: Vec::new(),
            subjects: Vec::new(),
        }),
        capture_unavailable: None,
        capture_observed_hour: None,
        capture_unavailable_hour: None,
        transfers: PeaceTransferGraph {
            nodes: Vec::new(),
            links: vec![PeaceTransferLink { id: format!("{id}:link"), source: String::new(), target: String::new(), state_count: states.len() as u32, victory_points: None, states }],
        },
        limitations: Vec::new(),
    }
}

fn reply(deals: Vec<PeaceDeal>) -> PeaceDealsReply {
    PeaceDealsReply {
        branch_id: "1".to_owned(),
        derived: false,
        deals,
        transfers: PeaceTransferGraph::default(),
        unobserved: PeaceUnobserved { ownership: vec![HourSpan { first_hour: 40, last_hour: 60 }], ..PeaceUnobserved::default() },
        warnings: Vec::new(),
    }
}

fn row<'a>(reply: &'a TerritoryTransfersReply, tag: &str) -> &'a TerritoryCountry {
    reply.countries.iter().find(|row| row.tag == tag).unwrap_or_else(|| panic!("{tag} has no row"))
}

#[test]
fn a_transfer_counts_once_published_and_its_victory_points_once_read_with_nothing_after_the_hour() {
    let (spr, spa, fin, sov) = (country(1, "SPR", 90), country(2, "SPA", 100), country(3, "FIN", 200), country(4, "SOV", 200));
    let peace = reply(vec![
        deal("civil", Some((100, 110)), vec![state(7, spr.clone(), spa.clone(), 100, Some("12"), Some(120)), state(8, spr, spa, 100, Some("3"), Some(100))]),
        deal("winter", None, vec![state(9, fin.clone(), sov.clone(), 200, None, None), state(9, sov, fin, 200, None, None)]),
    ]);

    let before = tally(&peace, 109);
    assert_eq!((before.transfers, before.countries.len(), before.next_change), (0, 0, Some(110)), "published at 110: unknown at 109");
    assert_eq!(before.unobserved, vec![HourSpan { first_hour: 40, last_hour: 60 }]);

    let published = tally(&peace, 110);
    assert_eq!(published.transfers, 2);
    let (taker, giver) = (row(&published, "SPA"), row(&published, "SPR"));
    assert_eq!((taker.gained, taker.lost, taker.net), (2, 0, 2));
    assert_eq!((giver.gained, giver.lost, giver.net), (0, 2, -2));
    assert_eq!(giver.tag, "SPR", "the giver keeps its own tag at its read");
    assert_eq!((taker.net_victory_points.as_deref(), taker.victory_points_missing.as_deref()), (None, Some(VP_NOT_YET_READ)), "one state's points read at 120");
    assert_eq!(published.next_change, Some(120));
    assert_eq!(published.countries.first().map(|row| row.tag.as_str()), Some("SPA"), "largest net gain first");

    let read = tally(&peace, 150);
    assert_eq!((row(&read, "SPA").net_victory_points.as_deref(), row(&read, "SPR").net_victory_points.as_deref()), (Some("15"), Some("-15")));
    assert_eq!(read.next_change, Some(200));

    let both = tally(&peace, 200);
    assert_eq!(both.transfers, 4, "a state transferred twice counts twice");
    let finland = row(&both, "FIN");
    assert_eq!((finland.gained, finland.lost, finland.net), (1, 1, 0));
    assert_eq!((finland.net_victory_points.as_deref(), finland.victory_points_missing.as_deref()), (None, Some(VP_NOT_RECORDED)));
    assert_eq!(both.next_change, None);
}
