use crate::save::{ActualCountry, CountryEmissionIdentity, SaveDocument};

const KEYS: [&[u8]; 2] = [b"fixture_r", b"fixture_a"];

fn document(countries: &str) -> SaveDocument {
    SaveDocument::read(
        format!("HOI4txt date=1936.1.1.1 game_unique_id=c version=b countries={{{countries}}}")
            .as_bytes(),
    )
    .unwrap()
}

fn units(fields: &str) -> String {
    [
        "units={ fleet={ id={id=1 type=61} task_force={id={id=2 type=61} mission={",
        fields,
        "} } } }",
    ]
    .concat()
}

#[test]
fn recorded_spellings_parse_exact_positive_values_and_preserve_each_omission() {
    for (fields, values) in [
        ("fixture_r=0.00001 fixture_a=7.125", [Some(1), Some(712500)]),
        ("fixture_a=7.125", [None, Some(712500)]),
        ("", [None, None]),
    ] {
        let saved = document(&format!("X={{{}}}", units(fields)))
            .mission_spotting_caches(KEYS)
            .unwrap();
        assert_eq!(
            saved[&((2u64 << 32) | 61)]
                .each_ref()
                .map(|value| value.as_ref().map(|value| value.raw)),
            values
        );
    }
}

#[test]
fn nonpositive_duplicate_fields_empty_and_aliased_bindings_refuse() {
    for fields in ["fixture_r=0", "fixture_a=-1", "fixture_r=1 fixture_r=2"] {
        assert!(
            document(&format!("X={{{}}}", units(fields)))
                .mission_spotting_caches(KEYS)
                .is_err()
        );
    }
    let document = document("X={}");
    for keys in [
        [b"".as_slice(), b"a".as_slice()],
        [b"r".as_slice(), b"r".as_slice()],
    ] {
        assert!(document.mission_spotting_caches(keys).is_err());
    }
}

#[test]
fn strict_country_view_and_duplicate_full_task_handle_are_not_deduplicated() {
    assert!(document("X={} X={}").mission_spotting_caches(KEYS).is_err());
    let task = units("fixture_r=1");
    assert!(
        document(&format!("X={{{task}}} Y={{{task}}}"))
            .mission_spotting_caches(KEYS)
            .is_err()
    );
}

#[test]
fn only_authenticated_equal_actual_country_emissions_share_cache_objects() {
    let task = units("fixture_r=1");
    let doc = document(&format!("X={{{task}}} X={{{task}}}"));
    let layout = [1, 2].map(|physical_slot| CountryEmissionIdentity {
        physical_slot,
        own_id: 1,
        actual: ActualCountry::Canonical(1),
        tag: b"X".to_vec(),
    });
    let saved = doc
        .project(&layout)
        .unwrap()
        .mission_spotting_caches(KEYS)
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert!(
        document(&format!("X={{{task}}} X={{{task} extra=1}}"))
            .project(&layout)
            .is_err()
    );
}
