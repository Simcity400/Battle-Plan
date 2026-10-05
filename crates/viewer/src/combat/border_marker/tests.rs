use super::*;
use crate::combat::binding::NoIcons;
use replay::rows::LandBorderWarSide;

fn localisation() -> Localisation {
    [
        (BORDER_CONFLICT_IF_YOU_WIN, "Win"),
        (BORDER_CONFLICT_IF_YOU_LOSE, "Lose"),
        (BORDER_CONFLICT_CHANGE_STATE, "$STATE$ to $COUNTRY$"),
        (BORDER_CONFLICT_FOR_ENEMY, "For $COUNTRY$"),
        (EFFECT_COUNTRY_EVENT, "Event $EVENT$"),
    ]
    .into_iter()
    .map(|(key, text)| (key.into(), text.into()))
    .collect()
}

#[test]
fn border_marker_tail_keeps_transfer_and_unbracketed_enemy_markers_in_native_order() {
    let border = LandBorderWarInputs {
        transfer_state: 128,
        sides: [
            LandBorderWarSide {
                country: 7,
                state: 11,
                win_event: b"own_win".to_vec(),
                lose_event: b"own_lose".to_vec(),
            },
            LandBorderWarSide {
                country: 9,
                state: 13,
                win_event: b"enemy_win".to_vec(),
                lose_event: b"enemy_lose".to_vec(),
            },
        ],
    };
    let mut body = "Marker".into();
    let mut refused = BTreeMap::new();
    build(
        &NoIcons,
        &localisation(),
        &border,
        0,
        &mut body,
        &|id, transfer| {
            Ok(country_marker(
                &format!("C{id}"),
                &format!("SYN{id}"),
                transfer,
            ))
        },
        &|id| Ok(format!("S{id}")),
        &|id, _| Description {
            body: format!("{}\n", String::from_utf8_lossy(id)),
            ..Default::default()
        },
        &mut refused,
    )
    .unwrap();
    assert_eq!(
        body,
        format!(
            "Marker\n\nWin\nS13 to C7 ({INLINE_FLAG}SYN7)\nown_win\nFor C9 {INLINE_FLAG}SYN9\nenemy_lose\n\nLose\nS11 to C9 ({INLINE_FLAG}SYN9)\nown_lose\nFor C9 {INLINE_FLAG}SYN9\nenemy_win\n"
        )
    );
    assert!(refused.is_empty());
    let mut body = "Marker".into();
    build(
        &NoIcons,
        &localisation(),
        &LandBorderWarInputs::default(),
        1,
        &mut body,
        &|_, _| unreachable!(),
        &|_| unreachable!(),
        &|_, _| unreachable!(),
        &mut refused,
    )
    .unwrap();
    assert_eq!(body, "Marker");
}

#[test]
fn border_marker_membership_and_event_scopes_use_native_alias_and_copy_links() {
    let links = [(7, 70), (9, 90)].into_iter().collect();
    assert_eq!(member(7, 70, &links), Ok(true));
    assert_eq!(member(9, 70, &links), Ok(false));
    assert_eq!(member(0, 70, &links), Ok(false));
    assert!(member(11, 70, &links).is_err());
    let values = scopes(70, 11, 90, 13).scopes;
    assert_eq!(values[0].root, Link::Scope(1));
    assert_eq!(values[0].from, Link::Scope(2));
    assert_eq!(values[2].from, Link::Scope(3));
    assert_eq!(values[3].from, Link::Scope(5));
    assert_eq!(values[5].from, Link::Itself);
    assert_eq!(values[0].prev, Link::Itself);
}

#[test]
fn border_marker_event_effects_or_title_follow_option_count_and_named_refusals() {
    let events = installed_content::events::parse(b"add_namespace=s country_event={id=s.1 title={text=IGNORED text=TITLE trigger={always=no}} immediate={custom_effect_tooltip=FIRST hidden_effect={custom_effect_tooltip=HIDDEN} add_political_power=2} option={name=A effect_tooltip={custom_effect_tooltip=SECOND}}}", "events/synthetic.txt", &installed_content::key_names::KeyNames::default());
    let mut event = events.get(b"S.01").unwrap().clone();
    let render = |key: &[u8]| Ok(format!("Text {}", String::from_utf8_lossy(key)));
    let text = event_description(&event, &render, &NoIcons, &localisation());
    assert_eq!(text.body, "Text FIRST\nText SECOND\n");
    assert_eq!(text.refused.values().sum::<usize>(), 1);
    assert!(
        text.refused
            .keys()
            .next()
            .unwrap()
            .contains("add_political_power")
    );
    event.options.push(Vec::new());
    // Native accepts the sole title before its trigger, even always=no.
    let text = event_description(&event, &render, &NoIcons, &localisation());
    assert_eq!(text.body, "Event Text TITLE");
    assert!(text.refused.is_empty());
    event.titles.push(event.titles[0].clone());
    let text = event_description(&event, &render, &NoIcons, &localisation());
    assert!(text.body.is_empty());
    assert!(
        text.refused
            .keys()
            .next()
            .unwrap()
            .contains("high-resolution seed")
    );
}

#[test]
fn border_marker_empty_enemy_description_has_no_enemy_heading_and_counts_failure() {
    let border = LandBorderWarInputs {
        sides: [
            LandBorderWarSide::default(),
            LandBorderWarSide {
                lose_event: b"failed".to_vec(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut body = String::new();
    let mut refused = BTreeMap::new();
    build(
        &NoIcons,
        &localisation(),
        &border,
        0,
        &mut body,
        &|_, _| unreachable!(),
        &|_| unreachable!(),
        &|_, _| Description {
            refused: [("synthetic refusal".into(), 3)].into_iter().collect(),
            ..Default::default()
        },
        &mut refused,
    )
    .unwrap();
    assert_eq!(body, "\nWin\n");
    assert_eq!(refused["synthetic refusal"], 3);
}

#[test]
fn event_coverage_note_keeps_text_and_operative_reader_refusal_still_suppresses_it() {
    let events = installed_content::events::parse(
        b"add_namespace=s country_event={id=s.1 immediate={custom_effect_tooltip=TEXT}} operative_leader_event={id=s.2 title=TITLE}",
        "events/synthetic.txt",
        &installed_content::key_names::KeyNames::default(),
    );
    let note = "events/synthetic.txt:1: UNVERIFIED event_content_coverage";
    let render = |key: &[u8]| Ok(String::from_utf8_lossy(key).into_owned());
    let mut event = events.get(b"s.1").unwrap().clone();
    event.notes.push(note.into());
    let text = event_description(&event, &render, &NoIcons, &localisation());
    assert_eq!(text.body, "TEXT\n");
    assert_eq!(text.refused[note], 1);
    let mut event = events.get(b"s.2").unwrap().clone();
    event.notes.push(note.into());
    let text = event_description(&event, &render, &NoIcons, &localisation());
    assert!(text.body.is_empty());
    assert_eq!(text.refused[note], 1);
    assert_eq!(text.refused.values().sum::<usize>(), 2);
    assert!(
        text.refused
            .keys()
            .any(|key| key.contains("operative_leader_event"))
    );
}
