use super::*;
use replay_frame::frame::DecodedSource;

#[test]
fn decoded_source_keeps_effect_text_bytes_and_dormant_refusals() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let fixture = Fixture::new();
    let source: &dyn DecodedSource = &fixture.frame;
    let scripted = fixture.catalogue.scripted_content().unwrap();
    let text = ScriptedText::new(source, &fixture.catalogue.localisation, scripted, 1);
    text.set_synthetic_world(World::read(
        source,
        ContentFacts::content_only(&fixture.dlcs, &fixture.triggers, &fixture.aliases),
    ));
    let context = Context::new(
        &text,
        &fixture.catalogue.localisation,
        1,
        "synthetic",
        Content::default(),
    );
    let body = content_reader_parser::parser::parse(
        b"if={limit={always=no} unported_effect=1} else={add_political_power=3 army_experience=2}",
    )
    .unwrap();
    let description = describe(&body.items, &context, &mut fresh());
    assert_eq!(description.text.as_bytes(), b"PP3\nXP2\n");
    assert!(
        description.refusals.is_empty(),
        "{:?}",
        description.refusals
    );
}
