use super::*;
use crate::products::peace::{PeaceCaptured, PeaceDates, PeaceObservation, PeaceTransferredState};
use dashboard_base::span::HourSpan;

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
    }
}

fn deal(id: &str, captured: Option<(i32, i32)>, states: Vec<PeaceTransferredState>) -> PeaceDeal {
    PeaceDeal {
        id: id.to_owned(),
        derived: captured.is_none(),
        observation: PeaceObservation::RecordedConference,
        conference_id: None,
        first_observed_hour: 0,
        last_observed_hour: 0,
        first_finalized_hour: None,
        end_hour: None,
        end_is_by: false,
        first_absent_hour: None,
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
        transfers: states,
        limitations: Vec::new(),
    }
}

fn index(deals: Vec<PeaceDeal>) -> TransfersIndex {
    let unobserved = PeaceUnobserved { ownership: vec![HourSpan { first_hour: 400, last_hour: 460 }], ..PeaceUnobserved::default() };
    let warnings = vec![
        PeaceWarning { hour: Some(300), text: "late".to_owned() },
        PeaceWarning { hour: None, text: "always".to_owned() },
    ];
    TransfersIndex::new("1".to_owned(), &deals, &unobserved, warnings)
}

fn row<'a>(reply: &'a TerritoryTransfersReply, tag: &str) -> &'a TerritoryCountry {
    reply.countries.iter().find(|row| row.tag == tag).unwrap_or_else(|| panic!("{tag} has no row"))
}

#[test]
fn a_transfer_counts_once_published_and_its_victory_points_once_read_with_nothing_after_the_hour_and_counts_partial_from_a_gap() {
    let (spr, spa, fin, sov) = (country(1, "SPR", 90), country(2, "SPA", 100), country(3, "FIN", 200), country(4, "SOV", 200));
    let peace = index(vec![
        deal("civil", Some((100, 110)), vec![state(7, spr.clone(), spa.clone(), 100, Some("12"), Some(120)), state(8, spr, spa, 100, Some("3"), Some(100))]),
        deal("winter", None, vec![state(9, fin.clone(), sov.clone(), 200, None, None), state(9, sov, fin, 200, None, None)]),
    ]);

    let before = peace.answer(109);
    assert_eq!((before.transfers, before.countries.len(), before.next_change), (0, 0, Some(110)), "published at 110: unknown at 109");
    assert_eq!((before.partial.as_deref(), before.warnings.clone()), (None, vec!["always".to_owned()]), "a warning dated 300 is not known at 109");

    let published = peace.answer(110);
    assert_eq!(published.transfers, 2);
    let (taker, giver) = (row(&published, "SPA"), row(&published, "SPR"));
    assert_eq!((taker.gained, taker.lost, taker.net), (2, 0, Some(2)));
    assert_eq!((giver.gained, giver.lost, giver.net), (0, 2, Some(-2)));
    assert_eq!(giver.tag, "SPR", "the giver keeps its own tag at its read");
    assert_eq!((taker.net_victory_points.as_deref(), taker.victory_points_missing.as_deref()), (None, Some(VP_NOT_RECORDED)), "one state's points read at 120, not disclosed at 110");
    assert_eq!(published.next_change, Some(120));
    assert_eq!(published.countries.first().map(|row| row.tag.as_str()), Some("SPA"), "largest net gain first");

    let read = peace.answer(150);
    assert_eq!((row(&read, "SPA").net_victory_points.as_deref(), row(&read, "SPR").net_victory_points.as_deref()), (Some("15"), Some("-15")));
    assert_eq!(read.next_change, Some(200));

    let both = peace.answer(200);
    assert_eq!(both.transfers, 4, "a state transferred twice counts twice");
    let finland = row(&both, "FIN");
    assert_eq!((finland.gained, finland.lost, finland.net), (1, 1, Some(0)));
    assert_eq!((finland.net_victory_points.as_deref(), finland.victory_points_missing.as_deref()), (None, Some(VP_NOT_RECORDED)));
    assert_eq!(both.next_change, Some(300), "the dated warning is the next change");

    let warned = peace.answer(300);
    assert_eq!(warned.warnings, vec!["always".to_owned(), "late".to_owned()]);
    assert_eq!((warned.partial, warned.next_change), (None, Some(400)), "the coming unrecorded stretch bounds the proof before it starts");

    let gap = peace.answer(400);
    assert_eq!((gap.partial.as_deref(), gap.transfers, gap.next_change), (Some(PARTIAL), 4, None), "observed counts are kept, marked partial");
    let spain = row(&gap, "SPA");
    assert_eq!((spain.gained, spain.net, spain.net_victory_points.as_deref(), spain.victory_points_missing.as_deref()), (2, None, None, Some(PARTIAL)), "no exact net across a gap");
    let mut keys: Vec<&str> = gap.countries.iter().map(|row| row.key.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(gap.countries.iter().map(|row| row.key.as_str()).collect::<Vec<_>>(), keys, "unranked, in key order");
}

#[test]
fn a_country_is_named_by_its_latest_read_tag_through_the_hour_and_a_later_transfer_read_earlier_does_not_rename_it() {
    let giver = |tag: &str, hour: i32| country(5, tag, hour);
    let taker = country(6, "TAK", 0);
    let peace = index(vec![
        deal("first", Some((10, 10)), vec![state(1, giver("NEW", 100), taker.clone(), 10, None, None)]),
        deal("second", Some((20, 20)), vec![state(2, giver("OLD", 50), taker.clone(), 20, None, None)]),
        deal("third", Some((30, 30)), vec![state(3, giver("TIE", 100), taker, 30, None, None)]),
    ]);
    let name = |through: i32| {
        let reply = peace.answer(through);
        let row = reply.countries.iter().find(|row| row.key == "5:105").cloned();
        row.map(|row| (row.tag, row.hour, row.lost))
    };
    assert_eq!(name(9), None, "nothing is known before the first transfer");
    assert_eq!(name(10), Some(("NEW".to_owned(), 100, 1)));
    assert_eq!(name(20), Some(("NEW".to_owned(), 100, 2)), "a tag read at hour 50 does not replace one read at 100");
    assert_eq!(name(30), Some(("TIE".to_owned(), 100, 3)), "of two tags read at the same hour, the later transfer's is kept");
}
