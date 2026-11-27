use super::text::{Notices, Presenter};
use super::{
    EventFeedItem, EventFeedKind, EventFeedOption, EventFeedRow, EventFeedWindow, EventWindowDecision,
    EventWindowKind, EventWindowOption, EventWindowPresented, FeedEvent, tag,
};
use dashboard_base::queries::country_ids::Names;
use replay_data::event_feed_rows::{EventPresentation, WindowKind, WindowVerdict};
use replay_data::rows::Recorded;
use std::io;

pub(crate) const DECISION_NOT_RECORDED: &str = "the game's window decision was not recorded for this firing: the recording predates it, or its capture was withheld or incomplete that hour";
pub(crate) const PRESENTATION_NOT_RECORDED: &str = "the window's content was not recorded";

fn decision(verdict: WindowVerdict) -> EventWindowDecision {
    match verdict {
        WindowVerdict::Requested => EventWindowDecision::Requested,
        WindowVerdict::AiControlled => EventWindowDecision::AiControlled,
        WindowVerdict::HiddenOrAiControlled => EventWindowDecision::HiddenOrAiControlled,
        WindowVerdict::NotViewingCountry => EventWindowDecision::NotViewingCountry,
        WindowVerdict::SenderExcluded => EventWindowDecision::SenderExcluded,
        WindowVerdict::PlayerRestrictedNotMajor => EventWindowDecision::PlayerRestrictedNotMajor,
        WindowVerdict::PlayerRestrictedOptionTrigger => EventWindowDecision::PlayerRestrictedOptionTrigger,
        WindowVerdict::ExistingInstance => EventWindowDecision::ExistingInstance,
        WindowVerdict::AlreadyOpen => EventWindowDecision::AlreadyOpen,
    }
}

pub(crate) fn decided(event: &FeedEvent, kind: EventFeedKind) -> Option<EventFeedWindow> {
    if !matches!(kind, EventFeedKind::ScriptEventFired | EventFeedKind::NewsEventFired) {
        return None;
    }
    let decision = event.window.flatten().and_then(WindowVerdict::from_code).map(decision);
    Some(EventFeedWindow {
        instance: event.instance,
        decision,
        presented: None,
        missing: decision.is_none().then(|| DECISION_NOT_RECORDED.to_owned()),
    })
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn recorded(field: &Recorded<Vec<u8>>, name: &str, not_recorded: &mut Vec<String>) -> Option<String> {
    match field {
        Recorded::Value(bytes) => Some(text(bytes)),
        Recorded::NotRecorded => {
            not_recorded.push(format!("the {name} was not recorded"));
            None
        }
    }
}

fn not_offered(
    presentation: &EventPresentation,
    offered: Option<&[EventWindowOption]>,
    authored: Option<&[EventFeedOption]>,
    not_recorded: &mut Vec<String>,
) -> Option<Vec<EventFeedOption>> {
    let offered = offered?;
    let Some(authored) = authored else {
        not_recorded.push("options not offered need the event's recorded content, which did not resolve".to_owned());
        return None;
    };
    if authored.len() != presentation.option_count as usize {
        not_recorded.push(format!(
            "options not offered are not shown: the recorded content lists {} options and the live event held {}",
            authored.len(),
            presentation.option_count
        ));
        return None;
    }
    Some(
        authored
            .iter()
            .filter(|option| !offered.iter().any(|shown| shown.ordinal == Some(option.ordinal)))
            .cloned()
            .collect(),
    )
}

fn shown(presentation: &EventPresentation, authored: &EventFeedItem, not_recorded: &mut Vec<String>) -> EventWindowPresented {
    let title = recorded(&presentation.title, "title", not_recorded);
    let description = recorded(&presentation.description, "description", not_recorded);
    let (picture, picture_hidden) = match &presentation.picture {
        Recorded::Value(Some(key)) => (Some(text(key)), false),
        Recorded::Value(None) => (None, true),
        Recorded::NotRecorded => {
            not_recorded.push("the picture was not recorded".to_owned());
            (None, false)
        }
    };
    let options = match &presentation.options {
        Recorded::Value(options) => Some(
            options
                .iter()
                .map(|option| EventWindowOption {
                    ordinal: option.ordinal,
                    label: option.label.as_deref().map(text),
                })
                .collect::<Vec<_>>(),
        ),
        Recorded::NotRecorded => {
            not_recorded.push("the offered options were not recorded".to_owned());
            None
        }
    };
    if options.iter().flatten().any(|option| option.label.is_none()) {
        not_recorded.push("an offered option's label was not recorded".to_owned());
    }
    if presentation.shown.is_none() {
        not_recorded.push("whether the window was shown was not recorded".to_owned());
    }
    if presentation.native_hour.is_none() {
        not_recorded.push("the game clock at the build was not recorded; the row stands at its read hour".to_owned());
    }
    let not_offered = not_offered(presentation, options.as_deref(), authored.options.as_deref(), not_recorded);
    EventWindowPresented {
        kind: match presentation.kind {
            WindowKind::Normal => EventWindowKind::Normal,
            WindowKind::News => EventWindowKind::News,
            WindowKind::Leader => EventWindowKind::Leader,
            WindowKind::Operative => EventWindowKind::Operative,
        },
        shown: presentation.shown,
        rebuilt: presentation.rebuild,
        title,
        description,
        picture,
        picture_hidden,
        options,
        not_offered,
        not_recorded: not_recorded.clone(),
    }
}

pub(crate) fn presented(
    event: &FeedEvent,
    id: String,
    names: &Names,
    presenter: &Presenter<'_>,
    notices: &mut Notices,
) -> io::Result<Option<EventFeedRow>> {
    let actor = tag(names, event.actor.slot, event.hour);
    if actor.is_none() {
        notices.sampled("eventWindows", "Event windows", format!("The window built at {} names a country that holds no tag then.", event.hour), 1);
    }
    let mut row = EventFeedRow {
        id,
        hour: event.hour,
        read_hour: event.read_hour,
        name_hour: event.hour,
        exact: event.presentation.as_ref().is_some_and(|presentation| presentation.native_hour.is_some()),
        category: EventFeedKind::EventWindowPresented.category(),
        kind: EventFeedKind::EventWindowPresented,
        tags: actor.into_iter().collect(),
        participants: None,
        participants_missing: None,
        owner_missing: None,
        item: None,
        amount: None,
        cause: None,
        strike: None,
        decision_state: None,
        window: None,
    };
    let Some(presentation) = &event.presentation else {
        notices.unavailable("eventWindows", "Event windows", PRESENTATION_NOT_RECORDED, Some(1));
        row.window = Some(EventFeedWindow {
            instance: event.instance,
            decision: None,
            presented: None,
            missing: Some(PRESENTATION_NOT_RECORDED.to_owned()),
        });
        return Ok(Some(row));
    };
    let token = match &presentation.token {
        Recorded::Value(bytes) if !bytes.is_empty() => std::str::from_utf8(bytes).ok().map(str::to_owned),
        _ => None,
    }
    .or_else(|| event.native_event.map(|(_, id)| id.to_string()));
    let mut authored_notices = Notices::default();
    let authored = presenter.item(event, EventFeedKind::ScriptEventFired, token.clone(), &mut authored_notices)?;
    let mut not_recorded = Vec::new();
    let window = shown(presentation, &authored, &mut not_recorded);
    let icon = window
        .picture
        .as_deref()
        .zip(token.as_deref())
        .and_then(|(key, token)| presenter.picture(token, key, notices));
    row.item = Some(EventFeedItem {
        token,
        name: window.title.clone(),
        icon,
        description: window.description.clone(),
        options: window.options.as_ref().map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    option.ordinal.map(|ordinal| EventFeedOption {
                        ordinal,
                        name: option.label.clone(),
                    })
                })
                .collect()
        }),
        name_missing: window.title.is_none().then(|| "the window's title was not recorded".to_owned()),
        gaps: Vec::new(),
    });
    row.window = Some(EventFeedWindow {
        instance: Some(presentation.instance),
        decision: None,
        presented: Some(window),
        missing: None,
    });
    Ok(Some(row))
}
