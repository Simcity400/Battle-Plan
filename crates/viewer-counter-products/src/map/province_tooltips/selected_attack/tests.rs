use super::*;
use replay_data::{rows::DivisionRow, rows::TableRows};
use replay_derive::{attack_derive::NIGHT, attack_derive::TERRAIN};
use std::cell::RefCell;
use viewer_foundation::test_support::{test_frame, test_put as put};

#[test]
fn known_different_sources_skip_even_with_another_missing_location() {
    assert_eq!(
        same_current(&[Err("missing".into()), Ok(3), Ok(4)]),
        Ok(None)
    );
    assert_eq!(
        same_current(&[Err("missing".into()), Ok(3), Ok(3)]),
        Err("missing".into())
    );
    assert_eq!(same_current(&[Ok(3), Ok(3)]), Ok(Some(3)));
}

#[test]
fn the_representative_wrapper_is_the_census_standard_template() {
    let handle = (13_u64 << 32) | 52;
    let mut frame = test_frame(42);
    put(
        &mut frame,
        TableRows::Divisions(vec![
            DivisionRow {
                id: 7,
                template: handle,
                ..Default::default()
            },
            DivisionRow {
                id: 8,
                ..Default::default()
            },
        ]),
    );
    assert_eq!(template(&frame, 7), Ok(handle));
    assert_eq!(template(&frame, 8), Ok(0));
    assert!(template(&frame, 9).unwrap_err().contains("absent"));
}

#[test]
fn first_template_encounter_wins_and_known_null_wrappers_deduplicate() {
    let mut seen = BTreeSet::new();
    let mut unknown = false;
    assert_eq!(first_template(Ok(9), &mut seen, &mut unknown), Ok(Some(9)));
    assert_eq!(first_template(Ok(2), &mut seen, &mut unknown), Ok(Some(2)));
    assert_eq!(first_template(Ok(9), &mut seen, &mut unknown), Ok(None));
    assert_eq!(first_template(Ok(0), &mut seen, &mut unknown), Ok(Some(0)));
    assert_eq!(first_template(Ok(0), &mut seen, &mut unknown), Ok(None));
}

#[test]
fn unknown_earlier_wrapper_cannot_promote_a_later_representative() {
    let mut seen = BTreeSet::new();
    let mut unknown = false;
    assert_eq!(first_template(Ok(9), &mut seen, &mut unknown), Ok(Some(9)));
    assert_eq!(
        first_template(Err("missing wrapper".into()), &mut seen, &mut unknown),
        Err("missing wrapper".into())
    );
    assert_eq!(first_template(Ok(9), &mut seen, &mut unknown), Ok(None));
    assert!(first_template(Ok(2), &mut seen, &mut unknown).is_err());
    assert_eq!(first_template(Ok(2), &mut seen, &mut unknown), Ok(None));
}

#[test]
fn the_move_is_the_routes_last_edge_or_the_shared_province() {
    assert_eq!(retained_edge(5, &[]), None);
    assert_eq!(retained_edge(5, &[6]), Some((5, 6)));
    assert_eq!(retained_edge(5, &[4, 6]), Some((4, 6)));
    assert_eq!(retained_edge(5, &[4, 8, 6]), Some((8, 6)));
}

#[derive(Default)]
struct Asked {
    straits: Vec<u64>,
    attacks: Vec<(u64, u32, u32)>,
}

fn run(route: Fact<Vec<u32>>, blocked: bool) -> (Tooltip, Asked) {
    let mut out = Tooltip::default();
    let asked = RefCell::new(Asked::default());
    let template = |handle: u64| -> Fact<u64> { Ok(if handle == 2 { 20 } else { 10 }) };
    attack_slot(
        &mut out,
        &[1, 2, 3],
        &route,
        Slot {
            location: &|_| Ok(5),
            template: &template,
            blocked: &mut |handle, current| {
                assert_eq!(current, 5);
                asked.borrow_mut().straits.push(handle);
                Ok(blocked)
            },
            notice: &|| Ok("Blocked by strait".into()),
            attack: &|handle, source, target| {
                asked.borrow_mut().attacks.push((handle, source, target));
                Ok(Some(if handle == 2 {
                    Preview {
                        rows: vec![],
                        refusals: vec![("fort", "fort operands unread".into())],
                    }
                } else {
                    Preview {
                        rows: vec![
                            Row {
                                kind: TERRAIN,
                                value: -10_000,
                            },
                            Row {
                                kind: NIGHT,
                                value: 2_500,
                            },
                        ],
                        refusals: vec![],
                    }
                }))
            },
            heading: &|template| Ok(format!("Attack T{template}")),
            row: &|row| Ok(format!("{}: {}", row.kind, row.value)),
        },
    );
    (out, asked.into_inner())
}

#[test]
fn a_retained_route_lists_each_representatives_attack_modifiers() {
    let (out, asked) = run(Ok(vec![4, 6]), true);
    assert_eq!(asked.attacks, [(1, 4, 6), (2, 4, 6)]);
    assert!(asked.straits.is_empty());
    assert_eq!(
        out.title,
        format!("    Attack T10\n        {TERRAIN}: -10000\n        {NIGHT}: 2500")
    );
    assert_eq!(
        out.refused.keys().collect::<Vec<_>>(),
        [
            "province selected attack fort: fort operands unread",
            "province selected attack heading gate: the native modifier-list count is unavailable",
        ]
    );
    let (_, asked) = run(Ok(vec![6]), false);
    assert_eq!(asked.attacks, [(1, 5, 6), (2, 5, 6)]);
}

#[test]
fn an_empty_route_words_blocked_straits_and_an_unknown_route_decides_neither() {
    let (out, asked) = run(Ok(vec![]), true);
    assert_eq!(asked.straits, [1, 2]);
    assert!(asked.attacks.is_empty());
    assert_eq!(out.title, "    Blocked by strait\n    Blocked by strait");
    assert!(out.refused.is_empty());
    let (out, asked) = run(Ok(vec![]), false);
    assert_eq!(asked.straits, [1, 2]);
    assert!(out.title.is_empty());
    let (out, asked) = run(Err("search refused".into()), true);
    assert!(out.title.is_empty());
    assert!(asked.straits.is_empty() && asked.attacks.is_empty());
    assert_eq!(
        out.refused.keys().collect::<Vec<_>>(),
        ["province selected attack retained route: search refused"]
    );
}

#[test]
fn gated_lists_and_different_sources_add_nothing() {
    let mut out = Tooltip::default();
    attack_slot(
        &mut out,
        &[1, 2],
        &Err("unread route".into()),
        Slot {
            location: &|handle| Ok(handle as u32),
            template: &|_| panic!("no representative"),
            blocked: &mut |_, _| panic!("no strait"),
            notice: &|| panic!("no notice"),
            attack: &|_, _, _| panic!("no attack"),
            heading: &|_| panic!("no heading"),
            row: &|_| panic!("no row"),
        },
    );
    assert!(out.title.is_empty());
    assert!(out.refused.is_empty());
    let mut out = Tooltip::default();
    attack_slot(
        &mut out,
        &[1],
        &Ok(vec![6]),
        Slot {
            location: &|_| Ok(5),
            template: &|_| Ok(10),
            blocked: &mut |_, _| panic!("no strait"),
            notice: &|| panic!("no notice"),
            attack: &|_, _, _| Ok(None),
            heading: &|_| panic!("no heading"),
            row: &|_| panic!("no row"),
        },
    );
    assert!(out.title.is_empty() && out.refused.is_empty());
}
