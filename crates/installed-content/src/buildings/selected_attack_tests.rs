//! UNRUN. The native token/scan mapping remains held for this private source.
use super::*;

#[test]
fn signed_land_fort_weight_defaults_to_zero_and_follows_the_native_integer_scan() {
    let buildings = super::parse(
        b"@weight=-7 BUILDINGS={odd={LAND_FORT=@weight} empty={} word={land_fort=2147483648} scan={land_fort=010 land_fort=bad}}",
        &KeyNames::default()).unwrap();
    assert_eq!(buildings.types["odd"].land_fort, -7);
    assert_eq!(buildings.types["empty"].land_fort, 0);
    assert_eq!(buildings.types["word"].land_fort, i32::MIN);
    assert_eq!(buildings.types["scan"].land_fort, 8);
}

#[test]
fn differing_signed_fort_declarations_are_refused_but_identical_case_aliases_resolve() {
    let buildings = super::parse(
        b"buildings={odd={land_fort=2} odd={land_fort=-2}}",
        &KeyNames::default(),
    )
    .unwrap();
    assert!(buildings.by_token("ODD").is_err());
    let identical = super::parse(
        b"buildings={odd={land_fort=2} ODD={land_fort=2}}",
        &KeyNames::default(),
    )
    .unwrap();
    assert_eq!(identical.by_token("Odd").unwrap().unwrap().land_fort, 2);
}
