use super::*;

fn localisation(language: &str) -> Localisation {
    let mut localisation = Localisation::default();
    localisation.language = language.into();
    localisation
}

#[test]
fn retained_army_name_is_authoritative_without_generator_or_content() {
    for bytes in [
        b"Allocated legacy name".to_vec(),
        Vec::new(),
        b"Cache\0suffix".to_vec(),
    ] {
        let status = DivisionStatusRow {
            division: 7,
            name: Recorded::Value(Some(bytes.clone())),
            naming: Recorded::Value(None),
            ..Default::default()
        };
        assert_eq!(
            army_name(&status, &localisation("english"), "english", || panic!(
                "cached name does not acquire content"
            )),
            Ok(String::from_utf8(bytes).unwrap())
        );
    }
    let status = DivisionStatusRow {
        name: Recorded::Value(None),
        naming: Recorded::Value(Some(NameOperands {
            custom: b"Generator substitute".to_vec(),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(
        army_name(&status, &localisation("english"), "english", || panic!(
            "failed cache does not acquire content"
        ))
        .unwrap_err()
        .contains("DivisionStatus.name unavailable")
    );
}

#[test]
fn only_equal_nonprogrammatic_bytes_certify_cross_language_literal_names() {
    let mut status = DivisionStatusRow {
        name: Recorded::Value(Some(b"Player literal".to_vec())),
        naming: Recorded::Value(Some(NameOperands {
            custom: b"Player literal".to_vec(),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(
        army_name(&status, &localisation("french"), "english", || panic!(
            "literal needs no content"
        ))
        .as_deref(),
        Ok("Player literal")
    );
    for naming in [
        Recorded::Value(None),
        Recorded::NotRecorded,
        Recorded::Value(Some(NameOperands {
            custom: b"Different current literal".to_vec(),
            ..Default::default()
        })),
        Recorded::Value(Some(NameOperands {
            custom: b"Player literal".to_vec(),
            programmatic: 1,
            ..Default::default()
        })),
    ] {
        status.naming = naming;
        assert!(
            army_name(&status, &localisation("french"), "english", || panic!(
                "language refusal needs no content"
            ))
            .unwrap_err()
            .contains("stored in english")
        );
    }
    status.name = Recorded::Value(Some(Vec::new()));
    assert_eq!(
        army_name(&status, &localisation("french"), "english", || panic!(
            "empty needs no content"
        ))
        .as_deref(),
        Ok("")
    );
    status.name = Recorded::Value(Some(vec![0xff]));
    assert!(
        army_name(&status, &localisation("english"), "english", || panic!(
            "invalid text needs no content"
        ))
        .unwrap_err()
        .contains("not UTF-8")
    );
}

#[test]
fn only_historical_absence_uses_authored_name_selection() {
    let names = installed_content::division_names::parse(
        br#"G={ordered={2={"Generated %d"}}}"#,
        &Default::default(),
    )
    .unwrap();
    let mut status = DivisionStatusRow {
        name: Recorded::NotRecorded,
        naming: Recorded::Value(Some(NameOperands {
            group: b"G".to_vec(),
            order: 2,
            ordered: 1,
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(
        army_name(&status, &localisation("french"), "english", || Ok(&names)).as_deref(),
        Ok("Generated 2")
    );
    if let Recorded::Value(Some(naming)) = &mut status.naming {
        naming.order = 3;
    }
    assert!(
        army_name(&status, &localisation("english"), "english", || Ok(&names))
            .unwrap_err()
            .contains("legacy name allocation inputs")
    );
    status.name = Recorded::Value(Some(b"Retained allocation".to_vec()));
    assert_eq!(
        army_name(&status, &localisation("english"), "english", || panic!(
            "retained legacy cache needs no allocator"
        ))
        .as_deref(),
        Ok("Retained allocation")
    );
}
