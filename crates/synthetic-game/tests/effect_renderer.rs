use synthetic_game::Executable;

#[test]
fn changing_the_renderer_preserves_the_effect_token_and_both_readers() {
    let tables = Executable::empty()
        .effect_at(17, "if", 5, 7, 11)
        .effect_renderer("if", 37)
        .build();
    let registry = tables.registries.as_ref().unwrap();
    assert_eq!(registry.effects, [(17, "if")]);
    assert_eq!(registry.effect_classes, [(17, 5, 7)]);
    assert_eq!(
        registry.effect_identities,
        [(17, "synthetic_effect_class_25", 37)]
    );
}
