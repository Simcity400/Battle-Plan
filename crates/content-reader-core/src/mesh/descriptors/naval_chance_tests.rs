use super::*;

fn state(fields: &str) -> EntityState {
    let source = format!("entity = {{ name = e state = {{ name = normal {fields} }} }}");
    parse_descriptors(source.as_bytes(), "gfx/entities/chance.asset")
        .unwrap()
        .entities
        .remove("e")
        .unwrap()
        .states
        .remove(0)
}

#[test]
fn omitted_or_failed_chance_retains_stored_zero_and_legacy_default() {
    let omitted = state("");
    assert_eq!(omitted.chance, 1.0);
    assert_eq!(omitted.chance_weight, Some(0));
    assert_eq!(state("chance = invalid").chance_weight, Some(0));
    assert_eq!(
        state("chance = 0.5 chance = invalid").chance_weight,
        Some(0)
    );
    assert_eq!(state("chance = -0.5").chance_weight, Some(0));
}

#[test]
fn fractional_chance_truncates_at_five_digits_before_storage() {
    assert_eq!(state("chance = 0.5").chance_weight, Some(50_000));
    assert_eq!(state("chance = 0.000009").chance_weight, Some(0));
    assert_eq!(state("chance = 0.000019").chance_weight, Some(1));
    assert_eq!(
        state("chance = 1 chance = 0.25").chance_weight,
        Some(25_000)
    );
}

#[test]
fn positive_fixed5_chance_copies_its_low_signed_word() {
    assert_eq!(state("chance = 21474.83648").chance_weight, Some(i32::MIN));
    assert_eq!(state("chance = 42949.67297").chance_weight, Some(1));
}
