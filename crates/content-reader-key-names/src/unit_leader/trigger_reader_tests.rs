use super::*;
use content_reader_script::key_names::KeyNames;

#[test]
fn every_trait_trigger_skips_invalid_key_definitions() {
    content_reader_script::test_game::enter();
    for field_name in [
        "allowed",
        "prerequisites",
        "gain_xp",
        "gain_xp_leader",
        "unit_trigger",
        "country_trigger",
    ] {
        let text = format!(
            "{field_name} = {{ invalid = {{ @lost = 9 }} @kept = 3 always = yes }} kept = @kept lost = @lost"
        );
        let read = native::read(
            text.as_bytes(),
            native::Mode::KeepMark,
            native::Top::File(()),
            Some(&KeyNames::default()),
            &mut |(), field| {
                if field.keyword_is(field_name) {
                    trait_field(0, field.keyword().as_deref()).map(|_| ())
                } else {
                    Shape::One
                }
            },
        );
        assert!(read.unsettled.is_none(), "{field_name}: {read:?}");
        assert_eq!(
            read.items[1].value.scalar(),
            Some(b"3".as_slice()),
            "{field_name}"
        );
        assert_eq!(
            read.items[2].value.scalar(),
            Some(b"@lost".as_slice()),
            "{field_name}"
        );
    }
}

#[test]
fn unit_type_reads_keys_instead_of_guessing_a_keyless_list() {
    let read = native::read(
        b"unit_type = { ship @kept = 4 other = @kept } kept = @kept",
        native::Mode::KeepMark,
        native::Top::File(()),
        None,
        &mut |(), field| {
            if field.keyword_is("unit_type") {
                trait_field(0, field.keyword().as_deref()).map(|_| ())
            } else {
                Shape::One
            }
        },
    );
    assert!(read.unsettled.is_none(), "{read:?}");
    assert_eq!(read.items[1].value.scalar(), Some(b"@kept".as_slice()));
}
