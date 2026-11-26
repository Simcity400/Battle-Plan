
use super::{EventFeedCause, row::FeedEvent};
use replay_data::event_feed_rows::EventFeedRoute;

fn route(code: u8) -> EventFeedRoute {
    EventFeedRoute::ALL.into_iter().find(|route| *route as u8 == code).unwrap_or_default()
}

pub(super) fn words(event: &FeedEvent, ally: Option<String>) -> Option<EventFeedCause> {
    let mut words = event.cause.map(|cause| {
        let (how, within) = (route(cause.route), route(cause.within));
        let notes = [(is_join(how), "against every enemy of the ally's side"), (is_war_end(how), "with a truce"), (is_annexation(how), "taking over its units")];
        EventFeedCause {
            how: route_words(how).map(str::to_owned),
            within: route_words(within).map(str::to_owned),
            conference: cause.conference.map(|conference| conference.to_string()),
            state: cause.state,
            ally,
            reason: cause.reason.map(|reason| match reason {
                0 => "war".to_owned(),
                1 => "puppet".to_owned(),
                2 => "ally".to_owned(),
                3 => "asked to join".to_owned(),
                4 => "guarantee".to_owned(),
                5 => "not applicable".to_owned(),
                other => format!("native reason {other}"),
            }),
            notes: notes.into_iter().filter(|(held, _)| *held && cause.flags & 1 != 0).map(|(_, note)| note.to_owned()).collect(),
            selected_option: None,
            target_recorded: false,
        }
    });
    if event.scripted() {
        let words = words.get_or_insert_with(|| EventFeedCause { how: None, within: None, conference: None, state: None, ally: None, reason: None, notes: Vec::new(), selected_option: None, target_recorded: false });
        words.notes.extend(script_notes(event));
        words.selected_option = event.option;
        words.target_recorded = event.country_target.is_some() || event.numeric_target.is_some();
    }
    words
}

pub(super) fn script_notes(event: &FeedEvent) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some((kind, id)) = event.native_event {
        notes.push(format!("Native event identity: {id} (type {kind})."));
    }
    if let Some(id) = event.instance {
        notes.push(format!("Event instance: {id}."));
    }
    if let Some(option) = event.option {
        notes.push(format!("Selected native option ordinal: {option}."));
    }
    if let Some(state) = event.lifecycle {
        notes.push(format!("Native lifecycle result: {state}."));
    }
    if let Some(days) = event.days {
        notes.push(format!("Native duration result: {days} days."));
    }
    if let Some(target) = event.country_target {
        notes.push(format!("Country target reference: {target}."));
    }
    if let Some(target) = event.numeric_target {
        notes.push(format!("Numeric target: {target}."));
    }
    for (id, scope) in event.scopes.iter().flatten().enumerate() {
        notes.push(format!(
            "Scope {id}: country reference {}; state {}; ROOT {}, FROM {}, PREV {}; random words {}, {}.",
            scope.country, scope.state, scope.bindings[0], scope.bindings[1], scope.bindings[2], scope.random[0], scope.random[1]
        ));
        for (kind, reference) in ["character", "operation", "ace", "unit", "industrial organisation", "purchase contract", "raid", "project", "faction"].into_iter().zip(scope.references) {
            if reference != 0 {
                notes.push(format!("Scope {id} {kind} reference: {reference}."));
            }
        }
    }
    notes
}

pub(crate) fn civil_war(event: &FeedEvent) -> bool {
    let civil = EventFeedRoute::CivilWarAnnex as u8;
    event.cause.is_some_and(|cause| cause.route == civil || cause.within == civil)
}

fn is_join(route: EventFeedRoute) -> bool {
    (20..30).contains(&(route as u8))
}

fn is_war_end(route: EventFeedRoute) -> bool {
    (30..40).contains(&(route as u8))
}

fn is_annexation(route: EventFeedRoute) -> bool {
    (50..60).contains(&(route as u8))
}

fn route_words(route: EventFeedRoute) -> Option<&'static str> {
    use EventFeedRoute::*;
    Some(match route {
        None => return Option::None,
        DiplomaticCommand => "a diplomatic action",
        DeclareWarEffect => "a declare_war_on effect",
        CivilWarDeclaration => "a civil war's declaration",
        GiveGuaranteeEffect => "a give_guarantee effect",
        GiveMilitaryAccessEffect => "a give_military_access effect",
        EmbargoEffect => "an embargo effect",
        GiveMarketAccessEffect => "a market access effect",
        OfferJoinFaction => "an offer to join a faction",
        ConsoleDiplomacy | ConsoleCivilWar | ConsoleCivilWarSecond | ConsolePeace | ConsoleAnnex | ConsoleAnnexSecond | ConsoleStateOwner => "a console command",
        StartCivilWarEffect => "a start_civil_war effect",
        Coup => "a coup",
        DeclarationJoin => "its overlord's war declaration",
        AddToWarEffect => "an add_to_war effect",
        SubjectJoinsOverlord => "becoming a subject",
        OverlordJoinsSubject => "gaining a subject",
        JoinAlly => "a request to join an ally's war",
        CallAlly => "a call to arms",
        CivilWarTransferEnd => "a civil war taking over the old country's wars",
        AnnexEnd | StateAnnex => "an annexation",
        SubjectOverlordEnd => "becoming the subject of its enemy",
        SubjectWarsEnd => "a subject relation ending the subject's wars",
        StatusResetEnd => "a diplomatic status reset",
        PeaceEnd => "a peace conference",
        StartPeaceConferenceEffect => "a start_peace_conference effect",
        WhitePeaceEffect => "a white_peace effect",
        Surrender => "a capitulation",
        PeaceProposal => "an accepted peace proposal",
        PeaceApplication => "a peace conference's outcome",
        CivilWarAnnex => "a civil war's resolution",
        Integration => "integration at the lowest autonomy",
        BorderWarAnnex | BorderWarAnnexSecond | StateBorderWar | StateBorderWarSecond => "a border war",
        PeaceAnnex => "a peace conference annexing a loser left without states",
        AnnexEffect => "an annex_country effect",
        TransferStateToAnnex | TransferStateToEffect => "a transfer_state_to effect",
        TransferStateAnnex | TransferStateEffect => "a transfer_state effect",
        ReleasePuppetEffect => "a release_puppet effect",
        ReleasePuppetOnControlledEffect => "a release_puppet_on_controlled effect",
        ReleaseEffect => "a release effect",
        ReleaseOnControlledEffect => "a release_on_controlled effect",
        ReleaseAutonomyEffect => "a release_autonomy effect",
        ReleaseCommand => "a release command",
        StateOwnerFix => "the owner and controller consistency fix",
        SetStateOwnerEffect | SetStateOwnerToEffect => "a state owner effect",
        StatePeaceWinner => "a peace conference giving a winner its state",
        StateDaily => "the daily state update",
        StateRead => "loading a save",
        StateCivilWar | StateCivilWarSecond | StateCivilWarTransfer => "a civil war",
        StatePeaceLiberated => "a peace conference liberating a country",
        StateRelease => "a release",
        StatePeaceBeneficiary => "a peace conference giving a beneficiary its state",
        StateImpassable => "an impassable state following its neighbours",
        StateHistory => "a history owner change",
        Decision | TargetedDecision => "a decision",
        DecisionTimeout => "a decision timing out",
        ScriptedDiplomacy => "a scripted diplomatic action",
        OnAction => "an on_action",
    })
}
