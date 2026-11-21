use super::*;

#[test]
fn event_ids_survive_reading_and_clone_inheritance() {
    let descriptors = parse_descriptors(
        br#"
        entity = { name = scenario state = { name = default
            event = { time = 3.7 id = a_supply }
            event = { time = 4 id = "a_1_idle" }
        } }
        entity = { name = copy clone = scenario }
    "#,
        "fixture.asset",
    )
    .unwrap();
    let entity = descriptors.inherit("copy").unwrap();
    let events = &entity.states["default"][0].events;
    assert_eq!(events[0].id.as_deref(), Some("a_supply"));
    assert_eq!(events[0].time, 3.7);
    assert_eq!(events[1].id.as_deref(), Some("a_1_idle"));
}
